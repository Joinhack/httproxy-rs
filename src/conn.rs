use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use base64;
use bytes::BytesMut;
use httparse::{self, Status};
use std::mem::MaybeUninit;
use std::net::SocketAddr;

use super::Server;

const MIN_SIZE_HEADERS: usize = 32;
const MAX_SIZE_HEADERS: usize = 256;

const FORBIT: &[u8] =
    b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"Access Unauthorized\"\r\n";

pub(crate) struct Connect {
    stream: TcpStream,
    buf: BytesMut,
    server: Server,
    path_indices: (usize, usize),
    host_indices: HeaderIndices,
    header_indices: Option<Vec<HeaderIndices>>,
    http_version: u8,
    is_connect_method: bool,
    remote: Option<TcpStream>,
}

#[derive(Clone)]
struct HeaderIndices {
    name: (usize, usize),
    val: (usize, usize),
}

impl Connect {
    pub fn new(stream: TcpStream, server: Server) -> Self {
        let host_indices = HeaderIndices {
            name: (0, 0),
            val: (0, 0),
        };
        Connect {
            stream,
            server,
            host_indices,
            path_indices: (0, 0),
            header_indices: None,
            is_connect_method: false,
            http_version: 0,
            buf: BytesMut::with_capacity(1024),
            remote: None,
        }
    }

    pub async fn process_header(&mut self, addr: &SocketAddr) -> Result<(), String> {
        loop {
            let _ = match self.stream.read_buf(&mut self.buf).await {
                Ok(0) => {
                    return Err(format!("closed by client:{}", addr));
                }
                Ok(n) => n,
                Err(e) => return Err(e.to_string()),
            };
            let mut head_vec: Vec<httparse::Header<'_>>;
            let mut headers: [httparse::Header<'_>; MIN_SIZE_HEADERS] =
                unsafe { MaybeUninit::uninit().assume_init() };
            let mut req = httparse::Request::new(&mut headers);
            let bs = &self.buf.as_ref();
            let mut res = req.parse(bs);
            if matches!(res, Err(httparse::Error::TooManyHeaders)) {
                head_vec = vec![httparse::EMPTY_HEADER; MAX_SIZE_HEADERS];
                req = httparse::Request::new(&mut head_vec);
                res = req.parse(bs);
            }
            let _ = match res {
                Ok(Status::Partial) => continue,
                Ok(Status::Complete(n)) => n,
                Err(e) => {
                    return Err(e.to_string());
                }
            };
            println!("{}", unsafe { std::str::from_utf8_unchecked(bs) });
            self.is_connect_method = if let Some("CONNECT") = req.method {
                true
            } else {
                false
            };
            let mut header_indices: Vec<HeaderIndices> = vec![
                HeaderIndices {
                    name: (0, 0),
                    val: (0, 0),
                };
                req.headers.len()
            ];
            let path = req.path.unwrap();
            let path_start = path.as_ptr() as usize;
            let path_start = path_start - bs.as_ptr() as usize;
            let path_end = path_start + path.len();
            self.path_indices = (path_start, path_end);
            process_header_indices(bs, req.headers, &mut header_indices);
            self.header_indices = Some(header_indices);
            return Ok(());
        }
    }

    async fn handle_auth(&mut self) -> bool {
        let bs = &self.buf;
        let mut auth_header: Option<HeaderIndices> = None;
        for h in self.header_indices.as_ref().unwrap().iter() {
            let header = &bs[h.name.0..h.name.1];
            if header == &b"Host"[..] {
                self.host_indices = h.clone();
            }

            if header == &b"Authorization"[..] {
                auth_header = Some(h.clone());
            }
        }

        if self.server.get_username_ref().is_none() {
            return true;
        } else {
            if auth_header.is_some()  {
                let auth_header = auth_header.unwrap();
                if auth_header.val.1 - auth_header.val.0 > 6 {
                    let v = match base64::decode(&bs[auth_header.val.0 + 6..auth_header.val.1]) {
                        Ok(v) => v,
                        Err(_) => {
                            let _ = self.stream.write_all(FORBIT).await;
                            return false;
                        }
                    };
                    let user_passwd = unsafe { std::str::from_utf8_unchecked(&v) };
                    let user_passwd: Vec<&str> = user_passwd.split(':').collect();
                    if user_passwd.len() > 1 {
                        let username = self.server.get_username_ref().unwrap();
                        let password = match self.server.get_password_ref() {
                            Some(x) => x,
                            None => "",
                        };
                        if user_passwd[0] == username && user_passwd[1] == password {
                            return true;
                        }
                    }
                }
            }
            let _ = self.stream.write_all(FORBIT).await;
            return false;
        }
    }

    async fn handle_remote(&self) -> Option<TcpStream> {
        let bs = &self.buf;
        let (raw_host, remote) = self.get_host_port(bs);
        println!("remote {}", remote);
        let remote_stream = TcpStream::connect(remote)
            .await
            .expect(&format!("connect host:{} error", raw_host));
        Some(remote_stream)
    }

    fn get_host_port<'b>(&self, bs: &'b [u8]) -> (&'b str, String) {
        let raw_host = {
            if self.is_connect_method {
                unsafe {
                    let bs: &[u8] = &bs[self.path_indices.0..self.path_indices.1];
                    std::str::from_utf8_unchecked(bs)
                }
            } else {
                let host = &self.host_indices;
                let bs = &bs[host.val.0..host.val.1];
                unsafe { std::str::from_utf8_unchecked(bs) }
            }
        };

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

    pub async fn process_remote(&mut self, addr: SocketAddr) {
        if let Err(e) = self.process_header(&addr).await {
            eprintln!("{}", e);
            return;
        }
        if !self.handle_auth().await {
            return;
        }
        self.remote = self.handle_remote().await;
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
}

fn process_header_indices(bs: &[u8], head: &[httparse::Header<'_>], indices: &mut [HeaderIndices]) {
    let ptr = bs.as_ptr() as usize;
    for (idx, h) in indices.iter_mut().zip(head) {
        let name_start = h.name.as_ptr() as usize - ptr;
        idx.name = (name_start, name_start + h.name.len());
        let val_start = h.value.as_ptr() as usize - ptr;
        idx.val = (val_start, val_start + h.value.len());
    }
}
