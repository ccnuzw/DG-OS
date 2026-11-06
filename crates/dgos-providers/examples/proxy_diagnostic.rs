use std::time::Duration;
#[tokio::main]
async fn main() {
    let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url=format!("http://{}/models",listener.local_addr().unwrap());
    drop(listener);
    for direct in [false,true] {
        let mut builder=reqwest::Client::builder().timeout(Duration::from_secs(5));
        if direct {builder=builder.no_proxy();}
        match builder.build().unwrap().get(&url).send().await {
            Ok(r)=>println!("direct={direct} status={}",r.status().as_u16()),
            Err(e)=>println!("direct={direct} connect={} timeout={}",e.is_connect(),e.is_timeout()),
        }
    }
}
