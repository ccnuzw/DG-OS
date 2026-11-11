//! Every connection is made to a checked DNS snapshot. Automatic redirects,
//! proxies, authentication/cookies and decompression are disabled deliberately.
use super::{WebOutcome, WebSource, BRAVE_ENDPOINT};
use dgos_providers::SecretValue;
use futures::StreamExt;
use reqwest::{header, Client, Response};
use html5ever::{LocalName, tokenizer::{BufferQueue, Token, TokenSink, TokenSinkResult, Tokenizer, TagKind, states::RawKind}};
use serde::Deserialize;
use std::{cell::RefCell, net::{IpAddr, SocketAddr}, time::Duration};
use url::{Host, Url};

const MAX_BODY_BYTES: usize = 512 * 1024;
const MAX_SEARCH_BODY_BYTES: usize = 256 * 1024;
const MAX_PAGE_TEXT: usize = 12 * 1024;
const MAX_REDIRECTS: usize = 3;
const SEARCH_RESULTS: usize = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);

pub(super) fn public_url(input: &str) -> Result<Url, String> {
    if input.is_empty() || input.len() > 2048 || input.chars().any(|c| c.is_control() || c == '\\' || is_bidi(c)) {
        return Err("web_url_not_allowed".into());
    }
    let mut url = Url::parse(input).map_err(|_| "web_url_not_allowed")?;
    if url.scheme() != "https" || url.port_or_known_default() != Some(443)
        || !url.username().is_empty() || url.password().is_some()
    { return Err("web_url_not_allowed".into()); }
    match url.host().ok_or("web_url_not_allowed")? {
        Host::Ipv4(ip) if public_ip(IpAddr::V4(ip)) => {}
        Host::Ipv6(ip) if public_ip(IpAddr::V6(ip)) => {}
        Host::Domain(domain) => {
            let host = domain.trim_end_matches('.').to_ascii_lowercase();
            if host.is_empty() || !host.contains('.') || host.split('.').any(str::is_empty)
                || ["localhost", "local", "internal", "invalid", "test", "example", "onion", "home", "lan", "arpa"]
                    .iter().any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            { return Err("web_url_not_allowed".into()); }
        }
        _ => return Err("web_url_not_allowed".into()),
    }
    url.set_fragment(None);
    Ok(url)
}

pub(super) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0 || a == 10 || a == 127 || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || (b == 0 && (c == 0 || c == 2)) || (b == 88 && c == 99)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() { return public_ip(IpAddr::V4(mapped)); }
            let parts = ip.segments();
            // Global unicast only. Exclude special-use, transition and documentation ranges.
            (parts[0] & 0xe000) == 0x2000
                && !(parts[0] == 0x2001 && parts[1] < 0x0200)
                && !(parts[0] == 0x2001 && parts[1] == 0x0db8)
                && parts[0] != 0x2002
                && !(parts[0] == 0x3fff && (parts[1] & 0xf000) == 0)
        }
    }
}

fn checked_addresses(addresses: impl IntoIterator<Item = SocketAddr>) -> Result<Vec<SocketAddr>, String> {
    let mut result = Vec::new();
    for address in addresses {
        // Reject the whole answer, rather than picking a public address from a mixed answer.
        if address.port() != 443 || !public_ip(address.ip()) { return Err("web_address_not_public".into()); }
        if !result.contains(&address) { result.push(address); }
        if result.len() > 32 { return Err("web_dns_answer_limit".into()); }
    }
    if result.is_empty() { return Err("web_dns_unavailable".into()); }
    Ok(result)
}

async fn resolve(url: &Url) -> Result<Vec<SocketAddr>, String> {
    match url.host().ok_or("web_url_not_allowed")? {
        Host::Ipv4(ip) => checked_addresses([SocketAddr::new(IpAddr::V4(ip), 443)]),
        Host::Ipv6(ip) => checked_addresses([SocketAddr::new(IpAddr::V6(ip), 443)]),
        Host::Domain(host) => checked_addresses(tokio::net::lookup_host((host, 443)).await
            .map_err(|_| "web_dns_unavailable")?),
    }
}

fn pinned_client(url: &Url, addresses: &[SocketAddr]) -> Result<Client, String> {
    let builder = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .no_gzip().no_brotli().no_deflate().no_zstd()
        .connect_timeout(Duration::from_secs(8))
        .timeout(REQUEST_TIMEOUT)
        .user_agent("DGOS-Assistant-Web/1.0")
        .referer(false)
        .pool_max_idle_per_host(0);
    let builder = match url.host() {
        Some(Host::Domain(host)) => builder.resolve_to_addrs(host, addresses),
        // Numeric hosts connect directly to the same validated address, with no resolver call.
        _ => builder,
    };
    builder.build().map_err(|_| "web_transport_unavailable".into())
}

async fn body(response: Response, maximum: usize) -> Result<Vec<u8>, String> {
    if response.headers().get_all(header::CONTENT_ENCODING).iter().any(|value| {
        !value.to_str().is_ok_and(|value| value.trim().eq_ignore_ascii_case("identity"))
    }) { return Err("web_content_encoding_not_allowed".into()); }
    if response.content_length().is_some_and(|length| length > maximum as u64) { return Err("web_body_limit".into()); }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "web_response_interrupted")?;
        if chunk.len() > maximum.saturating_sub(bytes.len()) { return Err("web_body_limit".into()); }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) async fn fetch(input: &str) -> Result<WebOutcome, String> {
    tokio::time::timeout(REQUEST_TIMEOUT, fetch_inner(input)).await.map_err(|_| "web_timeout")?
}

async fn fetch_inner(input: &str) -> Result<WebOutcome, String> {
    let mut url = public_url(input)?;
    for redirects in 0..=MAX_REDIRECTS {
        let addresses = resolve(&url).await?;
        let client = pinned_client(&url, &addresses)?;
        let response = client.get(url.clone()).header(header::ACCEPT, "text/html, text/plain;q=0.9")
            .header(header::ACCEPT_ENCODING, "identity").send().await.map_err(|_| "web_request_failed")?;
        if response.status().is_redirection() {
            if redirects == MAX_REDIRECTS { return Err("web_redirect_limit".into()); }
            url = redirect_target(&url, response.headers().get(header::LOCATION)
                .and_then(|value| value.to_str().ok()).ok_or("web_redirect_invalid")?)?;
            continue;
        }
        if !response.status().is_success() { return Err("web_http_error".into()); }
        let content_type = response.headers().get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
            .ok_or("web_content_type_not_allowed")?;
        let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if mime != "text/html" && mime != "text/plain" { return Err("web_content_type_not_allowed".into()); }
        let bytes = body(response, MAX_BODY_BYTES).await?;
        // No browser execution, external subresources, stylesheet retrieval or persistent DOM.
        let (title, content, truncated) = tokio::task::spawn_blocking(move || {
            let text = String::from_utf8_lossy(&bytes);
            if mime == "text/html" {
                extract_html(&text, MAX_PAGE_TEXT)
            } else {
                let (content, truncated) = plain(&text, MAX_PAGE_TEXT);
                Ok((String::new(), content, truncated))
            }
        }).await.map_err(|_| "web_extraction_failed")??;
        if content.is_empty() { return Err("web_page_has_no_readable_text".into()); }
        let source = WebSource {
            id: uuid::Uuid::new_v4().to_string(), url: url.to_string(),
            title: if title.is_empty() { url.host_str().unwrap_or("Web page").into() } else { title },
            excerpt: plain(&content, 512).0, retrieved_at: chrono::Utc::now().timestamp(),
            kind: "web_page".into(), truncated,
        };
        return Ok(WebOutcome::new(vec![source], Some(content), truncated));
    }
    Err("web_redirect_limit".into())
}

fn redirect_target(from: &Url, location: &str) -> Result<Url, String> {
    if location.len() > 2048 || location.chars().any(|c| c.is_control() || c == '\\' || is_bidi(c)) {
        return Err("web_redirect_invalid".into());
    }
    let next = from.join(location).map_err(|_| "web_redirect_invalid")?;
    public_url(next.as_str())
}

#[derive(Deserialize)]
struct BraveResponse { web: Option<BraveWeb> }
#[derive(Deserialize)]
struct BraveWeb { #[serde(default)] results: Vec<BraveResult> }
#[derive(Deserialize)]
struct BraveResult { url: String, #[serde(default)] title: String, #[serde(default)] description: String }

pub(super) async fn search(query: &str, key: &SecretValue) -> Result<WebOutcome, String> {
    tokio::time::timeout(REQUEST_TIMEOUT, search_inner(query, key)).await.map_err(|_| "web_timeout")?
}

async fn search_inner(query: &str, key: &SecretValue) -> Result<WebOutcome, String> {
    // The token is attached only to this fixed official endpoint. Redirects are never followed.
    let url = public_url(BRAVE_ENDPOINT)?;
    let addresses = resolve(&url).await?;
    let client = pinned_client(&url, &addresses)?;
    let mut token = header::HeaderValue::from_str(key.expose()).map_err(|_| "web_api_key_invalid")?;
    token.set_sensitive(true);
    let response = client.get(url).query(&[("q", query), ("count", "5"), ("text_decorations", "false"), ("safesearch", "moderate")])
        .header("X-Subscription-Token", token)
        .header(header::ACCEPT, "application/json").header(header::ACCEPT_ENCODING, "identity")
        .send().await.map_err(|_| "web_search_request_failed")?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => return Err("web_search_key_rejected".into()),
        429 => return Err("web_search_rate_limited".into()),
        _ => return Err("web_search_http_error".into()),
    }
    let content_type = response.headers().get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("");
    if !content_type.split(';').next().is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json")) {
        return Err("web_search_response_invalid".into());
    }
    let bytes = body(response, MAX_SEARCH_BODY_BYTES).await?;
    let response: BraveResponse = serde_json::from_slice(&bytes).map_err(|_| "web_search_response_invalid")?;
    let results = response.web.map(|web| web.results).unwrap_or_default();
    let mut sources = Vec::new();
    let mut truncated = results.len() > SEARCH_RESULTS;
    // Search response URLs are untrusted too: validate before persisting a clickable source.
    for result in results.into_iter().take(SEARCH_RESULTS) {
        let Ok(url) = public_url(&result.url) else { truncated = true; continue; };
        if resolve(&url).await.is_err() { truncated = true; continue; }
        let (title, title_text, title_truncated) = extract_html(&result.title, 160)?;
        let (_, excerpt, excerpt_truncated) = extract_html(&result.description, 512)?;
        // HTML fragments don't have <title>; fragment text is the intended search title.
        let title = if title.is_empty() { title_text } else { title };
        sources.push(WebSource {
            id: uuid::Uuid::new_v4().to_string(), url: url.to_string(),
            title: if title.is_empty() { url.host_str().unwrap_or("Search result").into() } else { title },
            excerpt, retrieved_at: chrono::Utc::now().timestamp(), kind: "search_result".into(),
            truncated: title_truncated || excerpt_truncated,
        });
    }
    Ok(WebOutcome::new(sources, None, truncated))
}

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

pub(super) fn plain(input: &str, maximum: usize) -> (String, bool) {
    let mut output = String::new();
    let truncated = append_plain(&mut output, input, maximum);
    (output, truncated)
}

fn append_plain(output: &mut String, input: &str, maximum: usize) -> bool {
    for c in input.chars() {
        if is_bidi(c) || (c.is_control() && !c.is_whitespace()) { continue; }
        let c = if c.is_whitespace() { ' ' } else { c };
        if c == ' ' && (output.is_empty() || output.ends_with(' ')) { continue; }
        if output.len() + c.len_utf8() > maximum { return true; }
        output.push(c);
    }
    false
}

// A conservative preflight, not a second HTML parser. Only locate the end of
// potential tags, honoring quoted/unquoted attribute values. Bound the work
// before html5ever's quadratic duplicate-attribute checks can run.
fn bounded_tag(bytes: &[u8]) -> bool {
    #[derive(Clone, Copy)]
    enum State { Name, BeforeAttribute, Attribute, AfterAttribute, BeforeValue, Quoted(u8), Unquoted, AfterValue }
    let mut state = State::Name;
    for &byte in bytes.iter().take(8192) {
        let space = matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c);
        state = match state {
            State::Quoted(quote) => if byte == quote { State::AfterValue } else { state },
            _ if byte == b'>' => return true,
            State::Name => if space || byte == b'/' { State::BeforeAttribute } else { state },
            State::BeforeAttribute => if space || byte == b'/' { state } else { State::Attribute },
            State::Attribute => if space { State::AfterAttribute } else if byte == b'=' { State::BeforeValue } else if byte == b'/' { State::BeforeAttribute } else { state },
            State::AfterAttribute => if space { state } else if byte == b'=' { State::BeforeValue } else if byte == b'/' { State::BeforeAttribute } else { State::Attribute },
            State::BeforeValue => if space { state } else if byte == b'\'' || byte == b'"' { State::Quoted(byte) } else { State::Unquoted },
            State::Unquoted => if space { State::BeforeAttribute } else { state },
            State::AfterValue => if space || byte == b'/' { State::BeforeAttribute } else { State::Attribute },
        };
    }
    // An unfinished short tag is still bounded; the real parser handles it.
    bytes.len() < 8192
}

pub(super) fn extract_html(input: &str, maximum: usize) -> Result<(String, String, bool), String> {
    // Bound input tokens and attribute parsing before tokenization. Never build
    // a DOM: HTML formatting reconstruction can amplify a tiny input into
    // millions of nodes even when input tags and nesting are bounded.
    let bytes = input.as_bytes();
    let mut starts = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte != b'<' { continue; }
        starts += 1;
        let rest = &bytes[index + 1..];
        let tag = rest.strip_prefix(b"/").unwrap_or(rest);
        if starts > 4096 || (tag.first().is_some_and(u8::is_ascii_alphabetic) && !bounded_tag(tag)) {
            return Err("web_html_complexity_limit".into());
        }
    }
    let sink = TextSink { state: RefCell::new(TextState::default()), maximum };
    let tokenizer = Tokenizer::new(sink, Default::default());
    let queue = BufferQueue::default();
    queue.push_back(input.into());
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    let state = tokenizer.sink.state.into_inner();
    Ok((state.title.trim().to_owned(), state.text.trim().to_owned(), state.truncated))
}

#[derive(Clone, Copy, Default)]
enum HeadState {
    #[default]
    Before,
    In(usize),
    After,
}
struct OpenTag {
    name: LocalName,
    hidden: bool,
    foreign: bool,
}
#[derive(Default)]
struct TextState {
    // Input tags only; no implicit elements or attribute cloning. Head state
    // records its stack boundary without allocating an implicit head/body.
    open: Vec<OpenTag>,
    head: HeadState,
    title: String,
    text: String,
    truncated: bool,
}
impl TextState {
    fn at_head(&self) -> bool {
        matches!(self.head, HeadState::In(depth) if self.open.len() == depth)
    }
}
struct TextSink { state: RefCell<TextState>, maximum: usize }
impl TokenSink for TextSink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut state = self.state.borrow_mut();
        match token {
            Token::CharacterTokens(value) => {
                if matches!(state.head, HeadState::Before) || state.at_head() {
                    // Only HTML whitespace can remain in the head. A body
                    // text token implicitly ends it, even without <body>.
                    if value.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}')) {
                        return TokenSinkResult::Continue;
                    }
                    state.head = HeadState::After;
                }
                if state.open.last().is_some_and(|tag| !tag.foreign && tag.name.as_ref() == "title") {
                    append_plain(&mut state.title, &value, 160);
                } else if !state.open.last().is_some_and(|tag| tag.hidden) {
                    state.truncated |= append_plain(&mut state.text, &value, self.maximum);
                }
            }
            Token::TagToken(tag) if tag.kind == TagKind::EndTag => {
                let foreign = state.open.last().is_some_and(|tag| tag.foreign);
                if !foreign {
                    if tag.name.as_ref() == "head" {
                        if state.at_head() { state.head = HeadState::After; }
                        return TokenSinkResult::Continue;
                    }
                    if matches!(state.head, HeadState::Before) || state.at_head() {
                        if !matches!(tag.name.as_ref(), "body" | "html" | "br") {
                            return TokenSinkResult::Continue;
                        }
                        state.head = HeadState::After;
                    }
                }
                let floor = match state.head { HeadState::In(depth) => depth, _ => 0 };
                // Opaque foreign content and head children cannot close an
                // outer HTML ancestor and thereby expose a hidden subtree.
                if let Some(index) = state.open.iter().enumerate().rev()
                    .take_while(|(index, open)| *index >= floor && open.foreign == foreign)
                    .find_map(|(index, open)| (open.name == tag.name).then_some(index))
                {
                    state.open.truncate(index);
                }
            }
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                let parent_foreign = state.open.last().is_some_and(|tag| tag.foreign);
                if !parent_foreign {
                    if name == "head" {
                        if matches!(state.head, HeadState::Before) {
                            state.head = HeadState::In(state.open.len());
                        }
                        return TokenSinkResult::Continue;
                    }
                    if name == "html" {
                        if !matches!(state.head, HeadState::Before) || !state.open.is_empty() {
                            return TokenSinkResult::Continue;
                        }
                    } else {
                        let head_content = matches!(name, "base" | "basefont" | "bgsound" | "link" | "meta" | "title" | "noscript" | "noframes" | "style" | "template" | "script");
                        if matches!(state.head, HeadState::Before) {
                            state.head = if head_content { HeadState::In(state.open.len()) } else { HeadState::After };
                        } else if state.at_head() && !head_content {
                            state.head = HeadState::After;
                        }
                    }
                }
                let foreign = parent_foreign || matches!(name, "svg" | "math");
                let hidden = state.open.last().is_some_and(|tag| tag.hidden)
                    || matches!(state.head, HeadState::In(_))
                    || matches!(name, "title" | "script" | "style" | "noscript" | "template" | "svg" | "math" | "iframe" | "object" | "embed" | "canvas" | "form" | "textarea")
                    || tag.attrs.iter().any(|attr| attr.name.local.as_ref() == "hidden" || (attr.name.local.as_ref() == "aria-hidden" && attr.value.as_ref() == "true"));
                if !hidden && matches!(name, "p" | "div" | "br" | "li" | "section" | "article" | "h1" | "h2" | "h3" | "tr") {
                    state.truncated |= append_plain(&mut state.text, " ", self.maximum);
                }
                let raw = if foreign && tag.self_closing { None } else { match name {
                    "script" => Some(RawKind::ScriptData),
                    "style" | "iframe" | "xmp" | "noembed" | "noframes" => Some(RawKind::Rawtext),
                    "title" | "textarea" => Some(RawKind::Rcdata),
                    _ => None,
                } };
                let void = !foreign && matches!(name, "area" | "base" | "basefont" | "bgsound" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr");
                // HTML non-void elements ignore a trailing slash. SVG/MathML
                // subtrees are hidden, but their self-closing tags must not
                // suppress the following HTML or enter an HTML raw-text mode.
                if !void && !(foreign && tag.self_closing) {
                    state.open.push(OpenTag { name: tag.name, hidden, foreign });
                }
                if let Some(raw) = raw { return TokenSinkResult::RawData(raw); }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn addresses_reject_private_special_mapped_and_mixed_dns() {
        for value in ["0.1.2.3", "10.0.0.1", "100.64.0.1", "127.0.0.1", "169.254.169.254", "172.16.1.1", "192.168.0.1", "192.0.2.1", "198.18.0.1", "198.51.100.1", "203.0.113.1", "224.0.0.1", "255.255.255.255", "::1", "fc00::1", "fe80::1", "::ffff:127.0.0.1", "64:ff9b::a00:1", "2001:db8::1", "2002:7f00:1::", "3fff::1"] {
            assert!(!public_ip(value.parse().unwrap()), "{value}");
        }
        for value in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111", "2001:4860:4860::8888", "::ffff:8.8.8.8"] {
            assert!(public_ip(value.parse().unwrap()), "{value}");
        }
        assert!(checked_addresses([SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8,8,8,8)),443), SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST),443)]).is_err());
    }

    #[test]
    fn url_and_redirect_rules_cannot_enter_local_or_authenticated_networks() {
        for url in ["http://example.org", "https://user:secret@example.org/", "https://example.org:444/", "https://localhost/", "https://metadata.google.internal/", "https://127.1/", "https://0x7f000001/", "https://[::ffff:127.0.0.1]/", "https://example.org\\@127.0.0.1/"] {
            assert!(public_url(url).is_err(), "{url}");
        }
        let from = public_url("https://example.org/path").unwrap();
        for target in ["http://example.org", "//127.0.0.1/", "https://user:secret@example.org/", "file:///etc/passwd", "//example.org:8443/"] {
            assert!(redirect_target(&from, target).is_err(), "{target}");
        }
        assert_eq!(redirect_target(&from, "/read?q=yes#frag").unwrap().as_str(), "https://example.org/read?q=yes");
    }

    #[test]
    fn extraction_drops_active_hidden_html_and_bounds_unicode() {
        let (title, text, truncated) = extract_html("<html><head><title>Safe &amp; title</title><style>secret-style</style></head><body><script>secret-script</script><template>secret-template</template><svg><text>secret-svg</text></svg><p hidden>secret-hidden</p><p aria-hidden='true'>secret-aria</p><p>Hello &amp; <b>world</b></p><p>next</p></body></html>", 100).unwrap();
        assert_eq!(title, "Safe & title");
        assert_eq!(text, "Hello & world next");
        assert!(!truncated);
        let (_, text, truncated) = extract_html("<p>你好世界</p>", 7).unwrap();
        assert_eq!(text, "你好");
        assert!(truncated);
        assert_eq!(plain("safe\u{202e}evil\0\ntext", 100).0, "safeevil text");
    }

    #[test]
    fn omitted_head_and_body_end_on_visible_elements_and_text() {
        for input in [
            "<html><head><title>T</title><p>Visible</p></html>",
            "<html><head><title>T</title>Visible</html>",
            "<html><title>T</title><p>Visible</p></html>",
            "<head><title>T</title></head><p>Visible</p>",
            "<html><head><head><html><meta charset='utf-8'><basefont/><bgsound/><title>T</title><p>Visible</p></html>",
        ] {
            let (title, text, truncated) = extract_html(input, 100).unwrap();
            assert_eq!(title, "T", "{input}");
            assert_eq!(text, "Visible", "{input}");
            assert!(!truncated, "{input}");
        }
    }

    #[test]
    fn implicit_head_exit_preserves_hidden_raw_and_template_content() {
        let input = "<html><head><title>T</title> \n<script>secret-script<p>still-script</script><style>secret-style</style><noscript><p>secret-noscript</p></noscript><template><html><head><body><p>secret-template</p></body></head></html></template><p hidden>secret-hidden</p><p aria-hidden='true'>secret-aria</p><p>Visible</p></html>";
        let (title, text, truncated) = extract_html(input, 100).unwrap();
        assert_eq!(title, "T");
        assert_eq!(text, "Visible");
        assert!(!truncated);
    }

    #[test]
    fn self_closing_foreign_content_does_not_hide_following_html() {
        for input in [
            "<body><svg/><p>Visible</p></body>",
            "<body><math/><p>Visible</p></body>",
            "<body><svg><g/><title/><style/><script/><text>secret-svg</text></svg><p>Visible</p></body>",
            "<body><math><mrow/><title/><style/><script/><mi>secret-math</mi></math><p>Visible</p></body>",
            "<body><div hidden><svg><g/></div><p>secret-foreign</p></svg></div><p>Visible</p></body>",
            "<body><svg><script>secret-script</svg><p>still-script</p></script></svg><p>Visible</p></body>",
            "<body><math><style>secret-style</math><p>still-style</p></style></math><p>Visible</p></body>",
        ] {
            let (title, text, truncated) = extract_html(input, 100).unwrap();
            assert_eq!(title, "", "{input}");
            assert_eq!(text, "Visible", "{input}");
            assert!(!truncated, "{input}");
        }
    }

    #[test]
    fn non_void_html_slashes_do_not_end_hidden_or_raw_elements() {
        for input in [
            "<body><div hidden/><p>secret-hidden</p></div><p>Visible</p></body>",
            "<body><div aria-hidden='true'/><p>secret-aria</p></div><p>Visible</p></body>",
            "<body><template/><p>secret-template</p></template><p>Visible</p></body>",
            "<body><script/>secret-script<p>still-script</script><p>Visible</p></body>",
            "<body><style/>secret-style<p>still-style</style><p>Visible</p></body>",
        ] {
            let (_, text, truncated) = extract_html(input, 100).unwrap();
            assert_eq!(text, "Visible", "{input}");
            assert!(!truncated, "{input}");
        }
    }

    #[test]
    fn implicit_body_and_foreign_self_closing_keep_existing_limits() {
        for prefix in ["<head><title>T</title>", "<body><svg/>", "<body><math/>"] {
            let (_, text, truncated) = extract_html(&format!("{prefix}<p>你好世界</p>"), 7).unwrap();
            assert_eq!(text, "你好", "{prefix}");
            assert!(truncated, "{prefix}");
        }
        let bounded = "<svg/>".repeat(4096) + "Visible";
        assert_eq!(extract_html(&bounded, MAX_PAGE_TEXT).unwrap(), (String::new(), "Visible".into(), false));
        assert_eq!(extract_html(&(bounded + "<math/>"), MAX_PAGE_TEXT).unwrap_err(), "web_html_complexity_limit");
    }

    #[test]
    fn deeply_nested_html_is_rejected_before_dom_construction() {
        let hostile = "<div>".repeat(100_000) + "visible text";
        assert!(hostile.len() < MAX_BODY_BYTES);
        assert_eq!(extract_html(&hostile, MAX_PAGE_TEXT).unwrap_err(), "web_html_complexity_limit");
        let bounded = "<div>".repeat(256) + "bounded visible text";
        assert_eq!(extract_html(&bounded, MAX_PAGE_TEXT).unwrap().1, "bounded visible text");
    }

    #[test]
    fn oversized_attribute_lists_and_quoted_delimiters_cannot_bypass_preflight() {
        let attributes = (0..70_000).map(|i| format!(" a{i:05}")).collect::<String>();
        let hostile = format!("<div{attributes}>visible text");
        assert!(hostile.len() < MAX_BODY_BYTES);
        assert_eq!(extract_html(&hostile, MAX_PAGE_TEXT).unwrap_err(), "web_html_complexity_limit");
        let quoted = format!("<div data-text='{}'>visible text", "<>".repeat(4090));
        assert_eq!(extract_html(&quoted, MAX_PAGE_TEXT).unwrap_err(), "web_html_complexity_limit");
        assert_eq!(extract_html("<div data-text='a > b' other=plain>visible text</div>", 100).unwrap().1, "visible text");
    }

    #[test]
    fn formatting_reconstruction_cannot_amplify_extraction() {
        let formatting = (0..2000).map(|i| format!("<b data-i='{i}'>")).collect::<String>();
        let input = format!("<p>{formatting}</p>{}", "<p>x</p>".repeat(1000));
        let (_, text, truncated) = extract_html(&input, MAX_PAGE_TEXT).unwrap();
        assert_eq!(text, "x ".repeat(999) + "x");
        assert!(!truncated);
    }

    #[tokio::test]
    async fn response_reader_rejects_encoding_and_oversized_bodies() {
        let mut server = mockito::Server::new_async().await;
        let encoded = server.mock("GET", "/encoded").with_header("content-encoding", "gzip")
            .with_body("not-decompressed-or-parsed").create_async().await;
        let oversized = server.mock("GET", "/large").with_body("x".repeat(1025)).create_async().await;
        // The response reader is tested in isolation; production still requires pinned HTTPS.
        let client = Client::builder().no_proxy().no_gzip().no_brotli().no_deflate().no_zstd().build().unwrap();
        let response = client.get(format!("{}/encoded", server.url())).send().await.unwrap();
        assert_eq!(body(response, 1024).await.unwrap_err(), "web_content_encoding_not_allowed");
        let response = client.get(format!("{}/large", server.url())).send().await.unwrap();
        assert_eq!(body(response, 1024).await.unwrap_err(), "web_body_limit");
        encoded.assert_async().await;
        oversized.assert_async().await;
    }
}
