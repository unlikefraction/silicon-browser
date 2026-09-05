use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct CapturedRequest {
    pub method: String,
    pub target: String,
    pub headers: String,
    pub body: Vec<u8>,
}

pub async fn spawn_json_server(
    responses: Vec<(u16, String)>,
) -> (String, mpsc::UnboundedReceiver<CapturedRequest>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (captured, requests) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            captured.send(request).unwrap();
            let reason = if (200..300).contains(&status) { "OK" } else { "ERROR" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        }
    });
    (format!("http://{address}/"), requests, server)
}

/// Respond once with headers only. Useful for proving that clients reject an advertised body
/// before attempting to buffer or decode it.
pub async fn spawn_header_only_server(
    status: u16,
    headers: Vec<(String, String)>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut stream).await;
        let reason = if (200..300).contains(&status) { "OK" } else { "ERROR" };
        let mut response = format!("HTTP/1.1 {status} {reason}\r\nConnection: close\r\n");
        for (name, value) in headers {
            response.push_str(&name);
            response.push_str(": ");
            response.push_str(&value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    (format!("http://{address}/"), server)
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> CapturedRequest {
    const MAX_REQUEST_BYTES: usize = 1024 * 1024;

    let mut bytes = Vec::new();
    let (header_end, content_length) = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "client closed before completing HTTP request");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() <= MAX_REQUEST_BYTES, "test HTTP request exceeded safety cap");
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            break (header_end, content_length);
        }
    };
    let body_start = header_end + 4;
    while bytes.len() < body_start + content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "client closed before completing HTTP body");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() <= MAX_REQUEST_BYTES, "test HTTP request exceeded safety cap");
    }

    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let request_line = headers.lines().next().unwrap();
    let mut parts = request_line.split_whitespace();
    CapturedRequest {
        method: parts.next().unwrap().into(),
        target: parts.next().unwrap().into(),
        headers,
        body: bytes[body_start..body_start + content_length].to_vec(),
    }
}
