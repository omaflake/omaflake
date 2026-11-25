// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ports of other VMs on this VM's localhost (plan B180): the Mac's welcome lists `forwards` – `{"port": 5432, "to":
//! "10.231.1.10:5432"}` – and the agent listens on 127.0.0.1:<port>, passing each connection on to its target in one
//! of omaflake's networks. A port stays bound once it was asked for; a later welcome without it closes connections
//! that come to it (the target is looked up per connection).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Forward {
    pub port: u16,
    pub to: String,
}

/// The ports listened on, with their targets now
#[derive(Default)]
pub struct Forwards {
    targets: Arc<Mutex<HashMap<u16, String>>>,
    listening: Vec<u16>,
}

impl Forwards {
    /// Takes the welcome's list: new ports get a listener, every port its target now
    pub fn set(&mut self, list: &[Forward], log: fn(&str)) {
        {
            let mut t = self.targets.lock().unwrap_or_else(|e| e.into_inner());
            t.clear();
            for f in list {
                t.insert(f.port, f.to.clone());
            }
        }
        for f in list {
            if self.listening.contains(&f.port) {
                continue;
            }
            match TcpListener::bind(("127.0.0.1", f.port)) {
                Ok(listener) => {
                    self.listening.push(f.port);
                    log(&format!("localhost:{} → {}", f.port, f.to));
                    let targets = self.targets.clone();
                    let port = f.port;
                    std::thread::spawn(move || serve(listener, port, targets, log));
                }
                Err(e) => log(&format!("localhost:{}: {e} – not forwarded", f.port)),
            }
        }
    }
}

fn serve(listener: TcpListener, port: u16, targets: Arc<Mutex<HashMap<u16, String>>>, log: fn(&str)) {
    for client in listener.incoming().flatten() {
        let to = targets.lock().unwrap_or_else(|e| e.into_inner()).get(&port).cloned();
        let Some(to) = to else {
            let _ = client.shutdown(Shutdown::Both);
            continue;
        };
        std::thread::spawn(move || {
            if let Err(e) = pass(client, &to) {
                log(&format!("localhost:{port} → {to}: {e}"));
            }
        });
    }
}

/// One connection to its target and back, until either side ends
fn pass(client: TcpStream, to: &str) -> std::io::Result<()> {
    let addr =
        to.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no address"))?;
    let server = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    let _ = client.set_nodelay(true);
    let _ = server.set_nodelay(true);
    let (c2, s2) = (client.try_clone()?, server.try_clone()?);
    let up = std::thread::spawn(move || copy(c2, s2));
    copy(server, client);
    let _ = up.join();
    Ok(())
}

fn copy(mut from: TcpStream, mut to: TcpStream) {
    let mut buf = [0u8; 65536];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.shutdown(Shutdown::Write);
    let _ = from.shutdown(Shutdown::Read);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet(_: &str) {}

    /// A port on localhost reaches its target, both ways; a port taken out of the list closes new connections
    #[test]
    fn localhost_reaches_the_target() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let to = target.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for mut s in target.incoming().flatten() {
                let mut b = [0u8; 5];
                s.read_exact(&mut b).unwrap();
                s.write_all(&[b"echo:" as &[u8], &b].concat()).unwrap();
            }
        });
        // A free port for the forward
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut f = Forwards::default();
        f.set(&[Forward { port, to }], quiet);
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.write_all(b"hello").unwrap();
        let mut back = [0u8; 10];
        c.read_exact(&mut back).unwrap();
        assert_eq!(&back, b"echo:hello");
        f.set(&[], quiet);
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut b = [0u8; 1];
        assert_eq!(c.read(&mut b).unwrap_or(0), 0, "closed: no target any more");
    }
}
