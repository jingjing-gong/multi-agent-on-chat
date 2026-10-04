//! Bounded local parser adapters; subprocess output is never interpreted as instructions.
use super::Asset;
use serde::{Deserialize, Serialize};
use std::{fmt, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub start_page: u32,
    pub end_page: Option<u32>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            start_page: 1,
            end_page: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaError {
    pub code: String,
    pub phase: String,
    pub message: String,
    pub retryable: bool,
}
impl MediaError {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            phase: "extract".into(),
            message: message.into(),
            retryable: false,
        }
    }
}
impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for MediaError {}
#[derive(Debug, Serialize, Deserialize)]
pub struct Segment {
    pub page: Option<u32>,
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub schema_version: u32,
    pub segments: Vec<Segment>,
    pub coverage: serde_json::Value,
    pub warnings: Vec<String>,
    pub truncated: bool,
}
async fn bounded<R: tokio::io::AsyncRead + Unpin>(
    stream: R,
    max: usize,
) -> Result<Vec<u8>, MediaError> {
    let mut bytes = Vec::new();
    stream
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| MediaError::new("parser_io", "cannot read parser output"))?;
    if bytes.len() > max {
        return Err(MediaError::new(
            "output_limit",
            "parser output exceeds budget",
        ));
    }
    Ok(bytes)
}
async fn run(
    program: &str,
    args: &[String],
    max: usize,
    seconds: u64,
) -> Result<Vec<u8>, MediaError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            MediaError::new("dependency_missing", "local PDF tools are not installed")
        } else {
            MediaError::new("parser_start_failed", "cannot start parser")
        }
    })?;
    let pid = child.id().unwrap_or(0);
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let output = tokio::time::timeout(Duration::from_secs(seconds), async {
        let capture=async {tokio::try_join!(bounded(out,max),bounded(err,16384))};
        tokio::pin!(capture);
        let mut monitor=tokio::time::interval(Duration::from_millis(100));
        let (out,err)=loop {tokio::select! {
            result=&mut capture=>break result?,
            _=monitor.tick()=> {
                let usage=Command::new("/bin/ps").args(["-o","rss=","-p",&pid.to_string()]).kill_on_drop(true).output().await.map_err(|_|MediaError::new("resource_monitor_failed","cannot monitor parser memory"))?;
                if let Ok(kib)=String::from_utf8_lossy(&usage.stdout).trim().parse::<u64>() {if kib>512*1024 {return Err(MediaError::new("memory_limit","parser exceeded 512 MiB resident memory"));}}
            }
        }};
        let status = child
            .wait()
            .await
            .map_err(|_| MediaError::new("parser_io", "parser wait failed"))?;
        if !status.success() {
            let error = String::from_utf8_lossy(&err);
            return Err(
                if error.to_ascii_lowercase().contains("password")
                    || error.to_ascii_lowercase().contains("encrypted")
                {
                    MediaError::new("encrypted_document", "PDF requires a password")
                } else {
                    MediaError::new("invalid_document", "PDF parser rejected the document")
                },
            );
        }
        Ok(out)
    })
    .await
    .map_err(|_| MediaError::new("parser_timeout", "local parsing exceeded time budget"))?;
    if output.is_err() {
        let _ = child.kill().await;
    }
    output
}
pub async fn identity(asset: &Asset) -> Result<String, MediaError> {
    match asset.mime.as_str() {
        "text/plain" => Ok("utf8-v1".into()),
        "application/pdf" => {
            let v = run("pdftotext", &["-v".into()], 16384, 5).await?;
            let info = run("pdfinfo", &["-v".into()], 16384, 5).await?;
            // Poppler commonly writes version to stderr; additionally fingerprint binaries by metadata.
            let mut fingerprint = format!(
                "poppler-v1:{}:{}",
                String::from_utf8_lossy(&v),
                String::from_utf8_lossy(&info)
            );
            for binary in ["pdftotext", "pdfinfo"] {
                if let Some(path) = std::env::var_os("PATH").and_then(|p| {
                    std::env::split_paths(&p)
                        .map(|d| d.join(binary))
                        .find(|p| p.is_file())
                }) {
                    if let Ok(m) = std::fs::metadata(path) {
                        fingerprint.push_str(&format!(":{}:{:?}", m.len(), m.modified().ok()));
                    }
                }
            }
            Ok(fingerprint)
        }
        _ => Err(MediaError::new(
            "parser_unavailable",
            "only UTF-8 text and PDF text extraction are supported",
        )),
    }
}
pub async fn extract(asset: &Asset, options: &Options) -> Result<ExtractionResult, MediaError> {
    let result = tokio::time::timeout(Duration::from_secs(60), extract_inner(asset, options)).await;
    result.map_err(|_| MediaError::new("parser_timeout", "extraction exceeded 60 seconds"))?
}
async fn extract_inner(asset: &Asset, options: &Options) -> Result<ExtractionResult, MediaError> {
    if options.start_page == 0
        || options
            .end_page
            .is_some_and(|e| e < options.start_page || e - options.start_page >= 50)
    {
        return Err(MediaError::new(
            "invalid_page_range",
            "select up to 50 pages",
        ));
    }
    if asset.mime == "text/plain" {
        if options.start_page != 1 || options.end_page.is_some() {
            return Err(MediaError::new(
                "invalid_page_range",
                "text files have no page ranges",
            ));
        }
        let file = tokio::fs::File::open(&asset.path)
            .await
            .map_err(|_| MediaError::new("source_unavailable", "cannot read source"))?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 4)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| MediaError::new("source_unavailable", "cannot read source"))?;
        let truncated = asset.size > 1024 * 1024;
        let n = bytes.len().min(1024 * 1024);
        bytes.truncate(n);
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(e) if truncated && e.utf8_error().error_len().is_none() => {
                String::from_utf8(e.as_bytes()[..e.utf8_error().valid_up_to()].to_vec()).unwrap()
            }
            Err(_) => {
                return Err(MediaError::new(
                    "invalid_encoding",
                    "file is not valid UTF-8",
                ))
            }
        };
        return Ok(ExtractionResult {
            schema_version: 1,
            segments: vec![Segment { page: None, text }],
            coverage: serde_json::json!({"source_bytes":asset.size,"extracted_bytes":n}),
            warnings: if truncated {
                vec!["text_limit_reached".into()]
            } else {
                vec![]
            },
            truncated,
        });
    }
    if asset.mime != "application/pdf" {
        return Err(MediaError::new("parser_unavailable", "unsupported format"));
    }
    let path = asset
        .path
        .to_str()
        .ok_or_else(|| MediaError::new("invalid_path", "source path is not UTF-8"))?
        .to_owned();
    let info = run("pdfinfo", std::slice::from_ref(&path), 65536, 10).await?;
    let info = String::from_utf8_lossy(&info);
    let total: u32 = info
        .lines()
        .find_map(|line| {
            line.strip_prefix("Pages:")
                .and_then(|s| s.trim().parse().ok())
        })
        .ok_or_else(|| MediaError::new("invalid_document", "missing PDF page count"))?;
    if options.start_page > total {
        return Err(MediaError::new(
            "invalid_page_range",
            "start page exceeds PDF length",
        ));
    }
    let end = options
        .end_page
        .unwrap_or(total.min(options.start_page.saturating_add(49)))
        .min(total);
    let mut segments = vec![];
    let mut warnings = vec![];
    let mut bytes = 0usize;
    let mut truncated = end < total || options.start_page > 1;
    for page in options.start_page..=end {
        let output = run(
            "pdftotext",
            &[
                "-f".into(),
                page.to_string(),
                "-l".into(),
                page.to_string(),
                "-enc".into(),
                "UTF-8".into(),
                "-layout".into(),
                path.clone(),
                "-".into(),
            ],
            1024 * 1024,
            15,
        )
        .await?;
        let text = String::from_utf8(output)
            .map_err(|_| MediaError::new("invalid_encoding", "PDF output is not UTF-8"))?;
        let text = text.trim_matches(|c: char| c.is_whitespace()).to_owned();
        if bytes + text.len() > 1024 * 1024 {
            warnings.push("text_limit_reached".into());
            truncated = true;
            break;
        }
        bytes += text.len();
        if text.is_empty() {
            warnings.push(format!("page_{page}_needs_ocr"));
        }
        segments.push(Segment {
            page: Some(page),
            text,
        });
    }
    if segments.iter().all(|s| s.text.trim().is_empty()) {
        return Err(MediaError::new(
            "needs_ocr",
            "selected PDF pages contain no extractable text; OCR is not enabled",
        ));
    }
    let actual_end = segments.last().and_then(|s| s.page);
    let processed = segments.len();
    Ok(ExtractionResult {
        schema_version: 1,
        segments,
        coverage: serde_json::json!({"total_pages":total,"start_page":options.start_page,"end_page":actual_end,"processed_pages":processed,"text_bytes":bytes}),
        warnings,
        truncated,
    })
}

pub fn capabilities() -> serde_json::Value {
    let installed = |name: &str| {
        std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|dir| dir.join(name).is_file()))
    };
    serde_json::json!({"pdftotext_installed":installed("pdftotext"),"pdfinfo_installed":installed("pdfinfo"),"execution":"local_subprocess","ocr":false,"limits":{"pages":50,"seconds":60,"text_bytes":1048576},"runtime_verification":"per_request"})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn subprocess_output_and_deadline_are_bounded() {
        let error = run("/bin/sh", &["-c".into(), "printf '123456789'".into()], 4, 5)
            .await
            .unwrap_err();
        assert_eq!(error.code, "output_limit");
        let error = run("/bin/sleep", &["5".into()], 100, 1).await.unwrap_err();
        assert_eq!(error.code, "parser_timeout");
    }
    #[tokio::test]
    async fn missing_dependency_is_structured() {
        let error = run("/nonexistent/mac-parser", &[], 100, 1)
            .await
            .unwrap_err();
        assert_eq!(error.code, "dependency_missing");
        assert!(!error.retryable);
    }
}
