// x402_fetch 가 보내는 요청과 받은 응답 (개발 77).
//
// 요청: method·headers·JSON body 를 한 덩이(`FetchRequest`)로 만든다. **첫 요청과 결제 헤더를 실은 재요청이 같은
// 덩이를 쓴다** — 사람이 승인한 결제는 「이 요청」에 대한 값이라, 재요청이 다른 method·body 로 나가면 서명한 것과
// 다른 걸 산다. 덩이를 두 번 만들지 않고 `apply` 하나로 두 요청을 조립하는 게 그 보장이다.
//
// 응답: 글자(JSON·텍스트)는 예전처럼 본문으로, 이미지·PDF 같은 바이너리는 파일로 저장하고 경로를 돌려준다.
// 작은 이미지는 MCP image content 로도 붙인다(main.rs). 글자라도 표시 상한을 넘으면 전체를 파일로 남긴다 —
// 돈을 내고 받은 응답이 잘려서 사라지면 안 된다.

use std::path::{Path, PathBuf};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::Method;

use crate::{tf, ts};

/// 보낼 JSON body 상한(바이트). 유료 API 요청 body 는 보통 수백 바이트다 — 증거 기록(`<id>.proof.json`)에도 실린다.
pub const MAX_REQUEST_BODY_BYTES: usize = 256 * 1024;

/// 받는 응답 상한(바이트) — 이걸 넘으면 그 자리에서 읽기를 멈춘다(메모리 보호). 이미지 생성 결과(수 MB)가 들어갈 크기.
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// 표시용 본문 문자 상한 — 에이전트/터미널 컨텍스트 보호. 넘으면 전체는 파일로 남긴다.
pub const MAX_BODY_CHARS: usize = 100_000;

/// MCP image content 로 붙일 이미지 상한(원본 바이트). base64 로 약 4/3 배가 된다. 넘으면 파일만.
pub const MAX_INLINE_IMAGE_BYTES: usize = 3 * 1024 * 1024;

/// 호출자가 지정할 수 없는 헤더(소문자). 결제 헤더는 지갑이 붙이고, 연결·길이 헤더는 HTTP 클라이언트가 정한다.
const RESERVED_HEADERS: &[&str] = &[
    "payment-signature",
    "x-payment",
    "payment-required",
    "payment-response",
    "x-payment-response",
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "te",
    "trailer",
    "upgrade",
    "expect",
];

/// 허용하는 method. 이 밖의 값은 막는다(HEAD 는 본문이 없어 결제할 대상이 없다).
const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE"];

/// 한 번 정하면 첫 요청·결제 재요청·증거 재제출이 **같은 값으로** 쓰는 요청.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchRequest {
    pub method: Method,
    /// 호출자가 준 헤더(검증 끝난 값). body 가 있고 Content-Type 을 안 줬으면 `application/json` 이 여기 들어 있다.
    pub headers: Vec<(HeaderName, HeaderValue)>,
    /// 직렬화한 JSON body(바이트 그대로 두 요청에 싣는다).
    pub body: Option<Vec<u8>>,
}

impl Default for FetchRequest {
    fn default() -> Self {
        Self::get()
    }
}

impl FetchRequest {
    /// 인자 없는 GET — 예전 x402_fetch 와 같다.
    pub fn get() -> Self {
        Self {
            method: Method::GET,
            headers: Vec::new(),
            body: None,
        }
    }

    /// 도구 인자에서 만든다. 못 읽는 값은 **막는다**(빼고 보내면 승인한 것과 다른 요청이 나간다).
    pub fn build(
        method: Option<&str>,
        headers: &[(String, String)],
        body: Option<&serde_json::Value>,
    ) -> Result<Self, String> {
        let method = match method {
            None => Method::GET,
            Some(m) => {
                let up = m.trim().to_ascii_uppercase();
                if !METHODS.contains(&up.as_str()) {
                    return Err(tf!(
                        "method 는 GET·POST·PUT·PATCH·DELETE 중 하나여야 해요: {m:?}",
                        "method must be one of GET, POST, PUT, PATCH, DELETE: {m:?}"
                    ));
                }
                Method::from_bytes(up.as_bytes()).map_err(|_| {
                    tf!(
                        "method 를 읽지 못했어요: {m:?}",
                        "Couldn't read the method: {m:?}"
                    )
                })?
            }
        };

        let body = match body {
            None | Some(serde_json::Value::Null) => None,
            Some(v @ serde_json::Value::Object(_)) => {
                if method == Method::GET {
                    return Err(ts!(
                        "GET 요청엔 body 를 실을 수 없어요. body 를 보내려면 method 를 POST 등으로 지정하세요.",
                        "A GET request can't carry a body. Set method (for example POST) to send one."
                    )
                    .into());
                }
                let bytes = serde_json::to_vec(v).map_err(|e| {
                    tf!(
                        "body 를 JSON 으로 만들지 못했어요: {e}",
                        "Couldn't serialize the body as JSON: {e}"
                    )
                })?;
                if bytes.len() > MAX_REQUEST_BODY_BYTES {
                    return Err(tf!(
                        "body 가 너무 커요({} 바이트, 최대 {}).",
                        "The body is too large ({} bytes, max {}).",
                        bytes.len(),
                        MAX_REQUEST_BODY_BYTES
                    ));
                }
                Some(bytes)
            }
            Some(_) => {
                return Err(ts!(
                    "body 는 JSON 객체({ ... })여야 해요.",
                    "body must be a JSON object ({ ... })."
                )
                .into())
            }
        };

        let mut out: Vec<(HeaderName, HeaderValue)> = Vec::new();
        for (k, v) in headers {
            let name = HeaderName::from_bytes(k.trim().as_bytes()).map_err(|_| {
                tf!(
                    "헤더 이름을 읽지 못했어요: {k:?}",
                    "Invalid header name: {k:?}"
                )
            })?;
            if RESERVED_HEADERS.contains(&name.as_str()) {
                return Err(tf!(
                    "이 헤더는 지갑이 정해요 — 빼고 다시 요청하세요: {}",
                    "This header is set by the wallet — leave it out and try again: {}",
                    name.as_str()
                ));
            }
            if out.iter().any(|(n, _)| *n == name) {
                return Err(tf!(
                    "같은 헤더가 두 번 왔어요: {}",
                    "The same header was given twice: {}",
                    name.as_str()
                ));
            }
            let value = HeaderValue::from_str(v.trim()).map_err(|_| {
                tf!(
                    "헤더 값을 읽지 못했어요({}): 줄바꿈·제어 문자는 넣을 수 없어요.",
                    "Invalid value for header {}: no line breaks or control characters.",
                    name.as_str()
                )
            })?;
            out.push((name, value));
        }
        if body.is_some() && !out.iter().any(|(n, _)| *n == CONTENT_TYPE) {
            out.push((CONTENT_TYPE, HeaderValue::from_static("application/json")));
        }

        Ok(Self {
            method,
            headers: out,
            body,
        })
    }

    /// 예전과 같은 「인자 없는 GET」인가 — 그때만 첫 요청이 리다이렉트를 따라간다(flow.rs).
    pub fn is_plain_get(&self) -> bool {
        self.method == Method::GET && self.headers.is_empty() && self.body.is_none()
    }

    /// 요청을 조립한다 — **첫 요청과 결제 재요청이 이 함수 하나로 만들어진다.**
    pub fn apply(&self, client: &reqwest::Client, url: &str) -> reqwest::RequestBuilder {
        let mut headers = HeaderMap::new();
        for (n, v) in &self.headers {
            headers.insert(n.clone(), v.clone());
        }
        let mut b = client.request(self.method.clone(), url).headers(headers);
        if let Some(body) = &self.body {
            b = b.body(body.clone());
        }
        b
    }

    /// 증거 기록(`<id>.proof.json`)에 남길 모양.
    pub fn to_stored(&self) -> StoredRequest {
        StoredRequest {
            method: self.method.as_str().to_string(),
            headers: self
                .headers
                .iter()
                .map(|(n, v)| {
                    (
                        n.as_str().to_string(),
                        String::from_utf8_lossy(v.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            body: self
                .body
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned()),
        }
    }

    /// 증거 기록에서 되살린다. 같은 검증을 다시 거친다 — 상한 파일을 그대로 믿지 않는다.
    pub fn from_stored(s: &StoredRequest) -> Result<Self, String> {
        let body: Option<serde_json::Value> = match &s.body {
            None => None,
            Some(text) => Some(serde_json::from_str(text).map_err(|e| {
                tf!(
                    "남은 요청 body 를 읽지 못했어요: {e}",
                    "Couldn't read the saved request body: {e}"
                )
            })?),
        };
        // 자동으로 붙인 Content-Type 도 기록엔 들어 있다 — 그대로 되살리면 같은 값이 된다.
        Self::build(Some(&s.method), &s.headers, body.as_ref())
    }
}

/// 증거 기록에 남는 요청(재제출이 같은 요청을 다시 보낼 수 있게, 개발 77).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct StoredRequest {
    pub method: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// JSON body 원문. 없으면 None.
    #[serde(default)]
    pub body: Option<String>,
}

/// MCP image content 로 붙일 이미지.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineImage {
    pub mime: String,
    pub data: Vec<u8>,
}

/// 받은 응답 — 글자 본문 + (있으면) 저장한 파일·붙일 이미지.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchedBody {
    /// AI/터미널에 보여 줄 글자. 바이너리면 저장 위치를 알리는 한 줄.
    pub text: String,
    /// 응답의 Content-Type(소문자, 파라미터 제외). 없으면 빈 값.
    pub content_type: String,
    /// 응답 크기(바이트, 읽은 만큼).
    pub bytes: usize,
    /// 파일로 저장했으면 그 경로. 빈 값 = 저장 안 함.
    pub saved_file: String,
    /// 작은 이미지면 MCP image content 로 붙일 원본.
    pub image: Option<InlineImage>,
}

impl FetchedBody {
    /// 글자 본문만 있는 응답(테스트·오류 표시용).
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            ..Self::default()
        }
    }
}

/// Content-Type 에서 파라미터를 뗀 소문자 mime.
pub fn mime_of(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// 글자로 보여 줄 mime 인가.
fn is_textual(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/json"
        || mime.ends_with("+json")
        || mime == "application/xml"
        || mime.ends_with("+xml")
        || mime == "application/javascript"
        || mime == "application/x-ndjson"
        || mime == "application/x-www-form-urlencoded"
}

/// MCP image content 로 붙일 수 있는 mime.
fn inline_image_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// 저장 파일 확장자 — 서버가 준 파일 이름은 쓰지 않는다(경로 조작 방지). 모르는 mime 은 `bin`.
fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/ogg" => "ogg",
        "video/mp4" => "mp4",
        "application/zip" => "zip",
        "text/csv" => "csv",
        "text/html" => "html",
        "text/markdown" => "md",
        m if m == "application/json" || m.ends_with("+json") => "json",
        m if m.starts_with("text/") => "txt",
        _ => "bin",
    }
}

/// 저장 폴더. `KURA_DOWNLOAD_DIR` > `~/Downloads/Kura` > `~/.jigap/downloads`.
fn download_dirs() -> Vec<PathBuf> {
    let mut dirs_out = Vec::new();
    if let Some(d) = std::env::var_os("KURA_DOWNLOAD_DIR").filter(|v| !v.is_empty()) {
        // 지정했으면 그 폴더만 쓴다 — 다른 곳으로 몰래 새지 않는다.
        dirs_out.push(PathBuf::from(d));
        return dirs_out;
    }
    if let Some(d) = dirs::download_dir() {
        dirs_out.push(d.join("Kura"));
    }
    if let Ok(j) = crate::wallet::jigap_dir() {
        dirs_out.push(j.join("downloads"));
    }
    dirs_out
}

/// 새 파일을 만들어 쓴다(덮어쓰지 않는다). 첫 폴더가 막히면(권한 등) 다음 폴더로.
fn save_bytes(bytes: &[u8], ext: &str) -> Result<PathBuf, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut last_err = ts!("저장할 폴더가 없어요", "No folder to save into").to_string();
    for dir in download_dirs() {
        for attempt in 0..8u32 {
            let name = format!(
                "kura-{}-{:06}{}.{ext}",
                now.as_secs(),
                now.subsec_micros(),
                if attempt == 0 {
                    String::new()
                } else {
                    format!("-{attempt}")
                }
            );
            match write_new(&dir, &dir.join(name), bytes) {
                Ok(p) => return Ok(p),
                Err(WriteErr::Exists) => continue,
                Err(WriteErr::Other(e)) => {
                    last_err = e;
                    break;
                }
            }
        }
    }
    Err(last_err)
}

enum WriteErr {
    Exists,
    Other(String),
}

fn write_new(dir: &Path, path: &Path, bytes: &[u8]) -> Result<PathBuf, WriteErr> {
    use std::io::Write;
    std::fs::create_dir_all(dir).map_err(|e| WriteErr::Other(e.to_string()))?;
    let mut f = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(WriteErr::Exists),
        Err(e) => return Err(WriteErr::Other(e.to_string())),
    };
    if let Err(e) = f.write_all(bytes) {
        let _ = std::fs::remove_file(path);
        return Err(WriteErr::Other(e.to_string()));
    }
    Ok(path.to_path_buf())
}

/// 응답 본문을 스트리밍으로 읽되 누적이 상한을 넘으면 즉시 멈춘다(Content-Length 없는 응답도 전량 버퍼링 안 함).
/// 반환: (읽은 바이트, 상한을 넘어 잘렸는지).
pub async fn read_limited(
    mut resp: reqwest::Response,
    cap: usize,
) -> Result<(Vec<u8>, bool), String> {
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| tf!("본문 읽기 실패: {e}", "Couldn't read the body: {e}"))?
    {
        if buf.len() + chunk.len() > cap {
            buf.extend_from_slice(&chunk[..cap - buf.len()]);
            return Ok((buf, true));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((buf, false))
}

/// 응답을 읽어 `FetchedBody` 로. **실패해도 Err 를 내지 않는다** — 결제 뒤에 부르는 자리라, 여기서 Err 가 나가면
/// 「돈은 나갔는데 결과가 오류」가 된다. 읽기·저장 실패는 글자 본문에 적는다.
pub async fn read_response(resp: reqwest::Response) -> FetchedBody {
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(mime_of)
        .unwrap_or_default();
    match read_limited(resp, MAX_RESPONSE_BYTES).await {
        Ok((bytes, truncated)) => classify(content_type, bytes, truncated),
        Err(e) => FetchedBody {
            text: format!("({e})"),
            content_type,
            ..FetchedBody::default()
        },
    }
}

/// 읽은 바이트를 글자/파일/이미지로 나눈다(네트워크 없이 테스트할 수 있게 따로 둔다).
pub fn classify(content_type: String, bytes: Vec<u8>, truncated: bool) -> FetchedBody {
    let len = bytes.len();
    let textual = if content_type.is_empty() {
        std::str::from_utf8(&bytes).is_ok()
    } else {
        is_textual(&content_type)
    };

    if textual {
        let full = String::from_utf8_lossy(&bytes).into_owned();
        let over_chars = full.chars().count() > MAX_BODY_CHARS;
        let mut text = if over_chars {
            full.chars().take(MAX_BODY_CHARS).collect()
        } else {
            full
        };
        let mut saved_file = String::new();
        if over_chars || truncated {
            if truncated {
                text.push_str(&tf!(
                    "\n…(응답이 {}MB를 넘어 거기까지만 받았어요)",
                    "\n…(the response was over {} MB — only that much was read)",
                    MAX_RESPONSE_BYTES / (1024 * 1024)
                ));
            }
            match save_bytes(&bytes, extension_for(&content_type)) {
                Ok(p) => {
                    saved_file = p.display().to_string();
                    text.push_str(&tf!(
                        "\n…(본문이 길어 잘랐어요 — 받은 전체는 파일에 있어요: {saved_file})",
                        "\n…(body truncated — everything received is in this file: {saved_file})"
                    ));
                }
                Err(e) => text.push_str(&tf!(
                    "\n…(본문이 길어 잘랐어요. 전체를 파일로 남기지 못했어요: {e})",
                    "\n…(body truncated, and saving the full body failed: {e})"
                )),
            }
        }
        return FetchedBody {
            text,
            content_type,
            bytes: len,
            saved_file,
            image: None,
        };
    }

    let shown_type = if content_type.is_empty() {
        "application/octet-stream".to_string()
    } else {
        content_type.clone()
    };
    // 상한을 넘은 바이너리는 앞부분만으로는 쓸 수 없다 — 저장하지 않고 그렇게 말한다.
    if truncated {
        return FetchedBody {
            text: tf!(
                "({shown_type} 응답이 {}MB를 넘어 받지 못했어요.)",
                "(The {shown_type} response was over {} MB, so it wasn't kept.)",
                MAX_RESPONSE_BYTES / (1024 * 1024)
            ),
            content_type,
            bytes: len,
            ..FetchedBody::default()
        };
    }
    let image =
        (inline_image_mime(&content_type) && len <= MAX_INLINE_IMAGE_BYTES).then(|| InlineImage {
            mime: content_type.clone(),
            data: bytes.clone(),
        });
    let (text, saved_file) = match save_bytes(&bytes, extension_for(&content_type)) {
        Ok(p) => {
            let path = p.display().to_string();
            (
                tf!(
                    "({shown_type} · {len} 바이트 — 파일로 저장했어요: {path})",
                    "({shown_type} · {len} bytes — saved to: {path})"
                ),
                path,
            )
        }
        Err(e) => (
            tf!(
                "({shown_type} · {len} 바이트 — 파일로 저장하지 못했어요: {e})",
                "({shown_type} · {len} bytes — couldn't save it to a file: {e})"
            ),
            String::new(),
        ),
    };
    FetchedBody {
        text,
        content_type,
        bytes: len,
        saved_file,
        image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(k: &str, v: &str) -> (String, String) {
        (k.into(), v.into())
    }

    #[test]
    fn default_is_plain_get() {
        let r = FetchRequest::build(None, &[], None).unwrap();
        assert_eq!(r, FetchRequest::get());
        assert!(r.is_plain_get());
    }

    #[test]
    fn post_json_body_gets_content_type() {
        let body = serde_json::json!({"query": "x402"});
        let r = FetchRequest::build(Some("post"), &[], Some(&body)).unwrap();
        assert_eq!(r.method, Method::POST);
        assert_eq!(r.body.as_deref(), Some(br#"{"query":"x402"}"#.as_slice()));
        assert_eq!(
            r.headers,
            vec![(CONTENT_TYPE, HeaderValue::from_static("application/json"))]
        );
        assert!(!r.is_plain_get());
        // 호출자가 준 Content-Type 은 그대로 — 덧붙이지 않는다.
        let r = FetchRequest::build(
            Some("POST"),
            &[h("Content-Type", "application/json; charset=utf-8")],
            Some(&body),
        )
        .unwrap();
        assert_eq!(r.headers.len(), 1);
        assert_eq!(r.headers[0].1, "application/json; charset=utf-8");
    }

    /// 못 읽는 값은 빼고 보내지 않고 막는다 — 빼면 승인한 것과 다른 요청이 나간다.
    #[test]
    fn unreadable_inputs_are_refused() {
        let obj = serde_json::json!({"a": 1});
        assert!(FetchRequest::build(Some("HEAD"), &[], None).is_err());
        assert!(FetchRequest::build(Some("TRACE"), &[], None).is_err());
        assert!(FetchRequest::build(Some(""), &[], None).is_err());
        assert!(FetchRequest::build(Some("GET"), &[], Some(&obj)).is_err());
        assert!(FetchRequest::build(None, &[], Some(&obj)).is_err()); // 기본 GET
        assert!(FetchRequest::build(Some("POST"), &[], Some(&serde_json::json!("text"))).is_err());
        assert!(FetchRequest::build(Some("POST"), &[], Some(&serde_json::json!([1]))).is_err());
        assert!(FetchRequest::build(Some("GET"), &[h("bad name", "v")], None).is_err());
        assert!(FetchRequest::build(Some("GET"), &[h("x-a", "a\r\nb")], None).is_err());
        assert!(FetchRequest::build(Some("GET"), &[h("X-A", "1"), h("x-a", "2")], None).is_err());
        let big = serde_json::json!({"s": "x".repeat(MAX_REQUEST_BODY_BYTES)});
        assert!(FetchRequest::build(Some("POST"), &[], Some(&big)).is_err());
        // null body 는 「없음」이다.
        assert!(
            FetchRequest::build(Some("POST"), &[], Some(&serde_json::Value::Null))
                .unwrap()
                .body
                .is_none()
        );
    }

    #[test]
    fn wallet_headers_cannot_be_set() {
        for name in [
            "PAYMENT-SIGNATURE",
            "X-Payment",
            "host",
            "Content-Length",
            "Transfer-Encoding",
        ] {
            assert!(
                FetchRequest::build(Some("GET"), &[h(name, "v")], None).is_err(),
                "{name} 를 받았다"
            );
        }
    }

    /// 증거 기록으로 남겼다 되살리면 같은 요청이다(재제출이 같은 요청을 보낸다).
    #[test]
    fn stored_round_trip() {
        let body = serde_json::json!({"prompt": "고양이", "n": 1});
        let r = FetchRequest::build(Some("POST"), &[h("x-mode", "fast")], Some(&body)).unwrap();
        let back = FetchRequest::from_stored(&r.to_stored()).unwrap();
        assert_eq!(back, r);
    }

    fn with_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
        // 테스트끼리 같은 환경변수를 쓴다 — 폴더는 하나로 두고 파일 이름은 매번 새로 만든다.
        let dir = std::env::temp_dir().join(format!("kura-fetch-test-{}", std::process::id()));
        std::env::set_var("KURA_DOWNLOAD_DIR", &dir);
        f(&dir)
    }

    #[test]
    fn json_stays_text() {
        let b = classify("application/json".into(), br#"{"ok":true}"#.to_vec(), false);
        assert_eq!(b.text, r#"{"ok":true}"#);
        assert!(b.saved_file.is_empty() && b.image.is_none());
    }

    #[test]
    fn image_is_saved_and_inlined() {
        with_dir(|dir| {
            let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
            let b = classify("image/png".into(), png.clone(), false);
            assert!(b.saved_file.ends_with(".png"), "{b:?}");
            assert!(Path::new(&b.saved_file).starts_with(dir));
            assert_eq!(std::fs::read(&b.saved_file).unwrap(), png);
            assert_eq!(b.image.as_ref().unwrap().mime, "image/png");
            assert!(b.text.contains(&b.saved_file));
        });
    }

    #[test]
    fn other_binary_is_saved_not_inlined() {
        with_dir(|_| {
            let b = classify("application/pdf".into(), b"%PDF-1.7".to_vec(), false);
            assert!(b.saved_file.ends_with(".pdf"));
            assert!(b.image.is_none());
            // 타입 없는 바이너리는 bin.
            let b = classify(String::new(), vec![0xff, 0xfe, 0x00], false);
            assert!(b.saved_file.ends_with(".bin"));
        });
    }

    #[test]
    fn truncated_binary_is_not_saved() {
        let b = classify("image/png".into(), vec![1, 2, 3], true);
        assert!(b.saved_file.is_empty() && b.image.is_none());
    }

    #[test]
    fn long_text_keeps_full_copy() {
        with_dir(|_| {
            let long = "가".repeat(MAX_BODY_CHARS + 10);
            let b = classify("text/plain".into(), long.clone().into_bytes(), false);
            assert!(!b.saved_file.is_empty());
            assert_eq!(std::fs::read_to_string(&b.saved_file).unwrap(), long);
            assert!(b.text.chars().count() < long.chars().count() + 200);
        });
    }

    #[test]
    fn mime_parsing() {
        assert_eq!(mime_of("Image/PNG; charset=binary"), "image/png");
        assert_eq!(extension_for("application/vnd.api+json"), "json");
        assert_eq!(extension_for("application/x-whatever"), "bin");
    }
}
