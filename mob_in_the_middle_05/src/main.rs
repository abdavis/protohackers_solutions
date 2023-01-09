use lazy_static::lazy_static;
use regex::Regex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("[::]:18534").await?;

    loop {
        let mut client_stream = BufReader::new(listener.accept().await?.0);

        tokio::spawn(async move {
            let mut server_stream =
                if let Ok(s) = TcpStream::connect("chat.protohackers.com:16963").await {
                    BufReader::new(s)
                } else {
                    return;
                };
            //let (server_rx, mut server_tx) = server_stream.split();
            //let (client_rx, mut client_tx) = client_stream.split();
            //let mut client_lines = BufReader::new(client_rx).lines();
            //let mut server_lines = BufReader::new(server_rx).lines();
            let mut server_buf = vec![];
            let mut client_buf = vec![];
            loop {
                tokio::select! {
                    client_msg = client_stream.read_until(b'\n', &mut client_buf) => match client_msg{
                        Err(e) => {
                            eprintln!("client read error: {e:?}");
                            break;
                        }
                        Ok(0) => break,
                        Ok(_) => {
                            let mut msg = String::from_utf8_lossy(&client_buf).into_owned();
                            replace_addresses(&mut msg);
                            if let Err(e) = server_stream.write_all(msg.as_bytes()).await{
                                eprintln!("server socket write error: {e:?}");
                                break;
                            }
                            client_buf.clear();
                        }

                    },
                    server_msg = server_stream.read_until(b'\n', &mut server_buf) => match server_msg{
                        Err(e) => {
                            eprintln!("server read error: {e:?}");
                            break;
                        }
                        Ok(0) => break,
                        Ok(_) => {
                            let mut msg = String::from_utf8_lossy(&server_buf).into_owned();
                            replace_addresses(&mut msg);
                            if let Err(e) = client_stream.write_all(msg.as_bytes()).await{
                                eprintln!("client socket write error: {e:?}");
                            }
                            server_buf.clear();
                        }
                    }
                }
            }
        });
    }
}

fn replace_addresses(input: &mut String) {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"(7[a-zA-Z0-9]{25,34})(?P<tail>$| |\n)").unwrap();
    }
    const TONYS_ADDRESS: &str = "7YWHMfk9JZe0LM0g1ZauHuiSxhI";
    *input = input
        .split_inclusive(' ')
        .map(|word| RE.replace(word, format!("{TONYS_ADDRESS}$tail")))
        .collect();
}
