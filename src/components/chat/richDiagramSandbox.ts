import { diagramInputFailure, MAX_RICH_OUTPUT_CHARS, RICH_RENDER_TIMEOUT_MS, type RichRenderFailure } from './richMarkdownSafety';

let library: Promise<string> | undefined;
function mermaidSource(): Promise<string> {
  // Required on-demand library loading: a static raw import would retain the entire Mermaid bundle at startup.
  // Importing dependency bytes also avoids ever executing Mermaid in the host origin.
  library ??= import('mermaid/dist/mermaid.min.js?raw').then(module => module.default).catch(error => { library = undefined; throw error; });
  return library;
}

function token(): string { return Array.from(crypto.getRandomValues(new Uint32Array(4)), value => value.toString(16).padStart(8, '0')).join(''); }

/** Untrusted source is delivered only as structured data after the opaque-origin frame is ready. */
function sandboxDocument(bundle: string, nonce: string, channel: string): string {
  const bootstrap = `(() => {
    const channel = ${JSON.stringify(channel)};
    let used = false;
    const send = value => parent.postMessage({channel, ...value}, '*');
    addEventListener('message', async event => {
      if (used || event.source !== parent || !event.data || event.data.channel !== channel || event.data.type !== 'render' || typeof event.data.source !== 'string' || event.data.source.length > 8000) return;
      used = true;
      try {
        mermaid.initialize({startOnLoad:false, securityLevel:'strict', maxTextSize:8000, maxEdges:100, suppressErrorRendering:true, htmlLabels:false, markdownAutoWrap:false, fontFamily:'sans-serif', flowchart:{htmlLabels:false, useMaxWidth:false}, sequence:{useMaxWidth:false}, secure:['securityLevel','startOnLoad','maxTextSize','maxEdges','htmlLabels','flowchart','sequence','theme','themeCSS','themeVariables']});
        const diagram = await mermaid.mermaidAPI.getDiagramFromText(event.data.source);
        const db = diagram.db;
        if (diagram.type === 'flowchart-v2' || diagram.type === 'flowchart') {
          if (db.getVertices().size > 60 || db.getEdges().length > 100 || db.getSubGraphs().length > 0) { send({failure:'limit'}); return; }
        } else if (diagram.type === 'sequence') {
          if (db.getActors().size > 60 || db.getMessages().length > 100) { send({failure:'limit'}); return; }
        } else { send({failure:'unsupported'}); return; }
        const result = await mermaid.render('diagram', event.data.source);
        send(result.svg.length > ${MAX_RICH_OUTPUT_CHARS} ? {failure:'limit'} : {markup:result.svg});
      } catch { send({failure:'failed'}); }
    });
    send({type:'ready'});
  })();`;
  // Only fixed dependency bytes enter script text; user Markdown never enters this document.
  const escapedBundle = bundle.replace(/<\/script/gi, '<\\/script');
  return `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-${nonce}'; style-src 'unsafe-inline'; img-src 'none'; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"></head><body><script nonce="${nonce}">${escapedBundle}</script><script nonce="${nonce}">${bootstrap}</script></body></html>`;
}

/** Removing a timed-out frame rejects late results. It is not a promise of CPU preemption on a shared renderer thread. */
export async function renderDiagram(source: string, signal: AbortSignal): Promise<string> {
  const failure = diagramInputFailure(source);
  if (failure) throw new Error(failure);
  if (signal.aborted) throw new Error('unavailable');
  // Keep the executor form for ES2020 WebViews without Promise.withResolvers.
  return new Promise((resolve, reject) => {
    const channel = token();
    const frame = document.createElement('iframe');
    let active = true;
    let sent = false;
    frame.setAttribute('sandbox', 'allow-scripts');
    frame.setAttribute('aria-hidden', 'true');
    frame.setAttribute('tabindex', '-1');
    frame.className = 'chat-rich-sandbox';
    frame.referrerPolicy = 'no-referrer';
    const finish = (error?: RichRenderFailure, markup?: string) => {
      if (!active) return;
      active = false;
      clearTimeout(timer);
      window.removeEventListener('message', receive);
      signal.removeEventListener('abort', abort);
      frame.remove();
      if (error) reject(new Error(error)); else resolve(markup!);
    };
    const receive = (event: MessageEvent<unknown>) => {
      if (!active || event.source !== frame.contentWindow || event.origin !== 'null' || typeof event.data !== 'object' || event.data === null) return;
      const data = event.data as { channel?: unknown; type?: unknown; markup?: unknown; failure?: unknown };
      if (data.channel !== channel) return;
      if (data.type === 'ready' && !sent) { sent = true; frame.contentWindow?.postMessage({ channel, type: 'render', source }, '*'); }
      else if (sent && typeof data.markup === 'string') finish(data.markup.length > MAX_RICH_OUTPUT_CHARS ? 'limit' : undefined, data.markup);
      else if (sent && (data.failure === 'failed' || data.failure === 'limit' || data.failure === 'unsupported')) finish(data.failure);
    };
    const abort = () => finish('unavailable');
    const timer = window.setTimeout(() => finish('timeout'), RICH_RENDER_TIMEOUT_MS);
    window.addEventListener('message', receive);
    signal.addEventListener('abort', abort, { once: true });
    void mermaidSource().then(bundle => {
      if (!active) return;
      frame.srcdoc = sandboxDocument(bundle, token(), channel);
      document.body.appendChild(frame);
    }).catch(() => finish('unavailable'));
  });
}
