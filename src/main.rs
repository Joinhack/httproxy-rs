use bytes::{BufMut, BytesMut};
use httparse::{self, Status};
use std::net::SocketAddr;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;

struct Connect<'h, 'b> {
    stream: &'b mut TcpStream,
    buf: &'b BytesMut,
    headers_len: usize,
    req: httparse::Request<'h, 'b>,
}

impl<'h, 'b> Connect<'h, 'b> {
    fn new(
        stream: &'b mut TcpStream,
        buf: &'b BytesMut,
        headers_len: usize,
        req: httparse::Request<'h, 'b>,
    ) -> Self {
        Connect {
            stream,
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
    writer.write_all(&buf[..]).await.unwrap();
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

async fn handle<'h, 'b>(conn: Connect<'h, 'b>, r: &mut Option<TcpStream>) {
    let req = &conn.req;
    let mut remote = if r.is_none() { None } else { r.take() };
    if remote.is_none() {
        let path: &str = req.path.unwrap();
        let (raw_host, remote_addr) = get_host_port(req);
        let n = path.find(raw_host).expect("not match host for path");
        let uri = &path[n + raw_host.len()..];
        let mut remote_stream = TcpStream::connect(remote_addr)
            .await
            .expect(&format!("connect host:{} error", raw_host));
        write_conn(&conn, uri, &mut remote_stream).await;
        remote = Some(remote_stream);
    }
    let mut remote_stream = remote.unwrap();
    let remaind = &conn.buf[conn.headers_len..];
    if remaind.len() > 0 {
        remote_stream.write_all(remaind).await.unwrap();
    }
    io::copy_bidirectional(conn.stream, &mut remote_stream)
        .await
        .unwrap();
}

async fn handle_conect<'h, 'b>(conn: Connect<'h, 'b>) -> Option<TcpStream> {
    let req = &conn.req;
    let (raw_host, remote) = get_host_port(req);
    let mut remote_stream = TcpStream::connect(remote)
        .await
        .expect(&format!("connect host:{} error", raw_host));
    conn.stream
        .write_all(format!(
            "HTTP/1.{} 200 Connection Established\r\n\r\n",
            req.version.unwrap()
        ).as_bytes())
        .await
        .unwrap();
    io::copy_bidirectional(conn.stream, &mut remote_stream)
        .await
        .unwrap();
    Some(remote_stream)
}

async fn process_client(mut tcp_stream: TcpStream, addr: SocketAddr) {
    let mut buf = BytesMut::with_capacity(1024);
    let mut remote: Option<TcpStream> = None;
    loop {
        let _ = match tcp_stream.read_buf(&mut buf).await {
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
            let conn = Connect::new(&mut tcp_stream, &buf, n, req);
            remote = handle_conect(conn).await;
            buf.clear();
        } else {
            let conn = Connect::new(&mut tcp_stream, &buf, n, req);
            handle(conn, &mut remote).await;
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
