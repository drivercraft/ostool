use std::{error::Error, fmt};

use reqwest::Response;

pub(crate) const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(crate) enum ReadBodyError {
    TooLarge { limit: usize },
    Read(reqwest::Error),
}

impl fmt::Display for ReadBodyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { limit } => write!(formatter, "body exceeds {limit} bytes"),
            Self::Read(error) => write!(formatter, "failed to read body: {error}"),
        }
    }
}

impl Error for ReadBodyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TooLarge { .. } => None,
            Self::Read(error) => Some(error),
        }
    }
}

pub(crate) async fn read_limited_body(
    mut response: Response,
    limit: usize,
) -> Result<Vec<u8>, ReadBodyError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(ReadBodyError::TooLarge { limit });
    }

    let capacity = response.content_length().unwrap_or(0).min(limit as u64) as usize;
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response.chunk().await.map_err(ReadBodyError::Read)? {
        let length = body
            .len()
            .checked_add(chunk.len())
            .ok_or(ReadBodyError::TooLarge { limit })?;
        if length > limit {
            return Err(ReadBodyError::TooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::{TcpListener, TcpStream},
        sync::oneshot,
    };

    use super::{ReadBodyError, read_limited_body};

    async fn read_request(socket: &mut TcpStream) {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            let read = socket.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
        }
    }

    async fn chunked_response(chunks: &[&str]) -> reqwest::Response {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = chunks
            .iter()
            .map(|chunk| format!("{:x}\r\n{chunk}\r\n", chunk.len()))
            .collect::<String>()
            + "0\r\n\r\n";
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{body}"
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        server.await.unwrap();
        response
    }

    #[tokio::test]
    async fn chunked_body_without_content_length_is_bounded_while_streaming() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket
                .write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\n\
                      transfer-encoding: chunked\r\n\
                      connection: close\r\n\r\n\
                      5\r\nhello\r\n6\r\nworld!\r\n",
                )
                .await
                .unwrap();
            let _ = release_rx.await;
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            read_limited_body(response, 6),
        )
        .await
        .expect("oversized streaming body must stop before the peer closes")
        .unwrap_err();
        assert!(matches!(error, ReadBodyError::TooLarge { limit: 6 }));
        release_tx.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn chunked_body_at_the_limit_is_returned_complete() {
        let response = chunked_response(&["hello", "!"]).await;
        assert_eq!(
            read_limited_body(response, 6).await.unwrap(),
            b"hello!".to_vec()
        );
    }
}
