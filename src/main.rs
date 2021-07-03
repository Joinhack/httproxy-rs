use bytes::BytesMut;
use httparse::{self, Status};
use std::net::SocketAddr;
use tokio::io::{self, AsyncReadExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};

async fn handle_get(req: &httparse::Request<'_, '_>) {
    let host = req
        .headers
        .iter()
        .rev()
        .find(|h| *h != &httparse::EMPTY_HEADER && h.name == "Host")
        .expect("no host found in header");
    let path: &str = req.path.unwrap();
    let raw_host = std::str::from_utf8(host.value).expect("error char encode for hosts");
    let hosts = raw_host.split(":").collect::<Vec<&str>>();
    let port = if hosts.len() == 2 {
        let port: i32 = hosts[1].parse().unwrap();
        port
    } else {
        80
    };
    let host = hosts[0];
    let n = path.find(raw_host).expect("not match host for path");
    let uri = &path[n + raw_host.len()..];
    let remote = format!("{}:{}", host, port);
    let remote_stream = TcpStream::connect(remote)
        .await
        .expect(&format!("connect host:{} error", raw_host));
    
    println!("{:?}", remote_stream);
}

async fn handle_conect() {}

async fn process_client(mut tcp_stream: TcpStream, addr: SocketAddr) {
    let mut buf = BytesMut::with_capacity(1024);
    loop {
        let n = match tcp_stream.read_buf(&mut buf).await {
            Ok(0) => {
                println!("closed by client:{}", addr);
                return;
            }
            Ok(n) => n,
            Err(_) => return,
        };
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        let n = match req.parse(&buf[..]) {
            Ok(Status::Partial) => continue,
            Ok(Status::Complete(n)) => n,
            Err(e) => {
                println!("{}", e);
                return;
            }
        };

        if let Some("GET") = req.method {
            handle_get(&req).await
        }
    }
}

async fn linstener_work(listener: &TcpListener) {
    loop {
        let (tcp_stream, addr) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            process_client(tcp_stream, addr).await;
        });
    }
}

fn main() {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
        linstener_work(&listener).await;
    });
}
