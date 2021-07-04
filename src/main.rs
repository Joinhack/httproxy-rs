use bytes::{BytesMut, BufMut};
use httparse::{self, Status};
use std::net::SocketAddr;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};

struct Connect<'h, 'b> {
    stream: &'b mut TcpStream,
    addr: SocketAddr,
    buf: &'b [u8],
    headers_len: usize,
    req: httparse::Request<'h, 'b>,
}

impl<'h, 'b> Connect<'h, 'b> {
    fn new(
        stream: &'b mut TcpStream,
        addr: SocketAddr,
        buf: &'b [u8],
        headers_len: usize,
        req: httparse::Request<'h, 'b>,
    ) -> Self {
        Connect {
            stream,
            addr,
            buf,
            headers_len,
            req,
        }
    }
}

async fn write_conn(conn: &Connect<'_, '_>, uri: &str, writer: &mut TcpStream) {
    let mut buf = BytesMut::with_capacity(1024);
    let req = &conn.req;
    buf.put_slice(req.method.unwrap().as_bytes());
    buf.put_slice(b" ");
    buf.put_slice(uri.as_bytes());
    buf.put_slice(b" ");
    buf.put_slice(format!("HTTP/1.{}\r\n", req.version.unwrap()).as_bytes());
    req.headers.iter().for_each(|x| {
        if x != &httparse::EMPTY_HEADER && x.name != "Proxy-Connection" {
            buf.put_slice(x.name.as_bytes());
            buf.put_slice(b": ");
            buf.put_slice(x.value);
            buf.put_slice(b"\r\n");
        }
    });
    buf.put_slice(b"\r\n");
    println!("{:?}", buf);
    writer.write_all(&buf[..]).await;
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

async fn handle<'h, 'b>(mut conn:  Connect<'h, 'b>) {
    let req = &conn.req;
    
    let path: &str = req.path.unwrap();
    let (raw_host, remote) = get_host_port(req);
    let n = path.find(raw_host).expect("not match host for path");
    let uri = &path[n + raw_host.len()..];
    let mut remote_stream = TcpStream::connect(remote)
        .await
        .expect(&format!("connect host:{} error", raw_host));
    println!("{:?}", remote_stream);
    write_conn(&conn, uri, &mut remote_stream).await;
    io::copy_bidirectional(conn.stream, &mut remote_stream).await;
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

        if let Some("CONNECT") = req.method {
        } else {
            let mut conn = Connect::new(&mut tcp_stream, addr, &buf[..], n, req);
            handle(conn).await
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
