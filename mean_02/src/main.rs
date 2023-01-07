use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("0:80").await?;

    loop {
        let (mut socket, _) = listener.accept().await?;

        tokio::spawn(async move {
            let mut buf = [0; 9];
            let mut map = BTreeMap::new();

            loop {
                match socket.read_exact(&mut buf).await {
                    Ok(_) => {
                        let ins = Instruction::from(&buf);
                        match ins.kind {
                            b'I' => {
                                map.insert(ins.timestamp_first, ins.price_last);
                            }
                            b'Q' => {
                                let average = if ins.timestamp_first > ins.price_last {
                                    0
                                } else {
                                    let mut count = 0i64;
                                    let mut sum = 0i64;
                                    for (_, val) in map.range(ins.timestamp_first..=ins.price_last)
                                    {
                                        count += 1;
                                        sum += *val as i64;
                                    }
                                    if sum == 0 {
                                        0
                                    } else {
                                        sum / count
                                    }
                                };
                                if let Err(e) = socket.write_i32(average as i32).await {
                                    eprintln!("socket write error: {e:?}");
                                }
                            }
                            _ => break,
                        }
                    }
                    Err(e) => {
                        eprintln!("socket read error: {e:?}");
                        break;
                    }
                }
            }
        });
    }
}
struct Instruction {
    kind: u8,
    timestamp_first: i32,
    price_last: i32,
}
impl From<&[u8; 9]> for Instruction {
    fn from(value: &[u8; 9]) -> Self {
        Self {
            kind: value[0],
            timestamp_first: i32::from_be_bytes(value[1..5].try_into().unwrap()),
            price_last: i32::from_be_bytes(value[5..].try_into().unwrap()),
        }
    }
}
