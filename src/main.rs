use bytes::{BufMut, BytesMut};
use httparse::{self, Status};
use std::net::SocketAddr;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;

struct Connect {
    stream: TcpStream,
    buf: BytesMut,
    http_version: u8,
    is_connect_method: bool,
    remote: Option<TcpStream>,
}

impl Connect {
    fn new(stream: TcpStream) -> Self {
        Connect {
            stream,
            is_connect_method: false,
            http_version: 0,
            buf: BytesMut::with_capacity(1024),
            remote: None,
        }
    }

    async fn process_remote(&mut self, addr: SocketAddr) {
        while self.remote.is_none() {
            let _ = match self.stream.read_buf(&mut self.buf).await {
                Ok(0) => {
                    println!("closed by client:{}", addr);
                    return;
                }
                Ok(n) => n,
                Err(_) => return,
            };
            let mut headers = [httparse::EMPTY_HEADER; 32];
            let mut req = httparse::Request::new(&mut headers);
            let _ = match req.parse(&self.buf[..]) {
                Ok(Status::Partial) => continue,
                Ok(Status::Complete(n)) => n,
                Err(e) => {
                    println!("{}", e);
                    return;
                }
            };
            self.http_version = req.version.unwrap();
            self.is_connect_method = if let Some("CONNECT") = req.method {
                 true
            } else {
                false
            };
            self.remote = Self::handle_connect(&req).await;
        }
        self.copy_bidirectional().await;
    }

    async fn copy_bidirectional(&mut self) {
        let stream: &mut TcpStream = &mut self.stream;
        let remote = self.remote.as_mut().unwrap();
        if self.is_connect_method {
            stream
                .write_all(
                    format!(
                        "HTTP/1.{} 200 Connection Established\r\n\r\n",
                        self.http_version
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        } else {
            remote.write_all(&self.buf[..]).await.unwrap();
        }
        let _ = io::copy_bidirectional(stream, remote).await.unwrap();
    }

    async fn handle_connect(req: &httparse::Request<'_, '_>) -> Option<TcpStream> {
        let (raw_host, remote) = Self::get_host_port(req);
        let remote_stream = TcpStream::connect(remote)
            .await
            .expect(&format!("connect host:{} error", raw_host));
        Some(remote_stream)
    }

    fn get_host_port<'h>(req: &httparse::Request<'h, '_>) -> (&'h str, String) {
        let host = req
            .headers
            .iter()
            .rev()
            .find(|h| *h != &httparse::EMPTY_HEADER && h.name == "Host")
            .expect("no host found in header");
        let raw_host = std::str::from_utf8(host.value).expect("error char encode for hosts");
        let hosts = raw_host.split(":").collect::<Vec<&str>>();
        let port = if hosts.len() == 2 {
            let port: i32 = hosts[1].parse().unwrap();
            port
        } else {
            80
        };
        let host = hosts[0];
        let remote = format!("{}:{}", host, port);
        (raw_host, remote)
    }
}

async fn linstener_work(listener: &TcpListener) {
    loop {
        let (tcp_stream, addr) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            let mut conn = Connect::new(tcp_stream);
            conn.process_remote(addr).await;
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
