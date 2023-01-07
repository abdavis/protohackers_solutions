use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::RwLock,
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("0:80").await?;

    let sieve = Arc::new(RwLock::new(Sieve::<1_000_000_000>::default()));
    loop {
        let (mut socket, _) = listener.accept().await?;
        let sieve = sieve.clone();
        tokio::spawn(async move {
            let (r, mut w) = socket.split();
            let mut buf_reader = BufReader::new(r);
            let mut buf = String::new();
            loop {
                println!("loop started");
                match buf_reader.read_line(&mut buf).await {
                    Ok(n) if n == 0 => {
                        println!("reached eof");
                        break;
                    }
                    Ok(_) => {
                        println!("read line from file");
                        match serde_json::from_str::<ClientMessage>(&buf) {
                            Ok(msg) if msg.method == "isPrime" => {
                                println!("got a message");
                                if let Number::U(n) = msg.number {
                                    let read_handle = sieve.read().await;
                                    if let Some(b) = read_handle.prime_check(n) {
                                        println!("already calculated this prime!: {n}:{b}");
                                        if let Err(e) = w
                                            .write_all(
                                                ServerMessage {
                                                    method: "isPrime".into(),
                                                    prime: b,
                                                }
                                                .json()
                                                .as_bytes(),
                                            )
                                            .await
                                        {
                                            eprintln!("error writing to socket: {e:?}");
                                            break;
                                        }
                                    } else {
                                        drop(read_handle);
                                        println!("finding primes up to {n}");
                                        if let Err(e) = w
                                            .write_all(
                                                ServerMessage {
                                                    method: "isPrime".into(),
                                                    prime: sieve.write().await.find_primes(n),
                                                }
                                                .json()
                                                .as_bytes(),
                                            )
                                            .await
                                        {
                                            eprintln!("error writing to socket: {e:?}");
                                            break;
                                        }
                                        println!("just sent a packet to client");
                                    }
                                } else {
                                    if let Err(e) = w
                                        .write_all(
                                            ServerMessage {
                                                method: "isPrime".into(),
                                                prime: false,
                                            }
                                            .json()
                                            .as_bytes(),
                                        )
                                        .await
                                    {
                                        eprintln!("Error writing to socket: {e:?}");
                                        break;
                                    }
                                }
                            }
                            _ => {
                                if let Err(e) = w
                                    .write_all(
                                        ServerMessage {
                                            method: "malformed".into(),
                                            prime: false,
                                        }
                                        .json()
                                        .as_bytes(),
                                    )
                                    .await
                                {
                                    eprintln!("error writing to socket: {e:?}");
                                    break;
                                }
                                eprintln!("malformed packet");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("socket read error: {e:?}");
                        break;
                    }
                }
                buf.clear();
            }
        });
    }
}
#[derive(Serialize)]
struct ServerMessage {
    method: String,
    prime: bool,
}
impl ServerMessage {
    fn json(&self) -> String {
        let mut out = serde_json::to_string(self).unwrap();
        out.push('\n');
        out
    }
}
#[derive(Deserialize)]
struct ClientMessage {
    method: String,
    number: Number,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Number {
    U(usize),
    I(isize),
    F(f64),
}

struct Sieve<const N: usize> {
    bools: [bool; N],
    last_prime: usize,
}

impl<const N: usize> Sieve<N> {
    fn prime_check(&self, num: usize) -> Option<bool> {
        if num > self.last_prime {
            None
        } else {
            Some(self.bools[num])
        }
    }
    fn find_primes(&mut self, num: usize) -> bool {
        while self.last_prime < num {
            for b in self.bools[self.last_prime..]
                .iter_mut()
                .step_by(self.last_prime)
                .skip(1)
            {
                *b = false;
            }
            let mut new_prime = self.last_prime;
            loop {
                new_prime += 1;
                if self.bools[new_prime] {
                    self.last_prime = new_prime;
                    break;
                }
            }
        }
        println!("done searching primes");
        self.bools[num]
    }
}

impl<const N: usize> Default for Sieve<N> {
    fn default() -> Self {
        let mut out = Self {
            bools: [true; N],
            last_prime: 2,
        };
        out.bools[0] = false;
        out.bools[1] = false;
        out
    }
}
