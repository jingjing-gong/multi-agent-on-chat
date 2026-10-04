//! Literal command output travels as a file when too large for a chat bubble.
use super::*;
use aes::cipher::{BlockEncrypt, KeyInit};

pub fn encrypt(data: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let padding = 16 - data.len() % 16;
    let mut bytes = data.to_vec();
    bytes.resize(data.len() + padding, padding as u8);
    let cipher = aes::Aes128::new(key.into());
    for block in bytes.as_chunks_mut::<16>().0 {
        cipher.encrypt_block(block.into());
    }
    bytes
}

pub async fn prepare(
    cred: &WechatCredential,
    target: &ReplyTarget,
    text: &str,
    id: &str,
) -> Result<Value> {
    prepare_bytes(cred, target, text.as_bytes(), "command-output.json", id).await
}
pub async fn prepare_bytes(
    cred: &WechatCredential,
    target: &ReplyTarget,
    data: &[u8],
    name: &str,
    id: &str,
) -> Result<Value> {
    let key = *uuid::Uuid::new_v4().as_bytes();
    let key_hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let encrypted = encrypt(data, &key);
    let filekey = uuid::Uuid::new_v4().simple().to_string();
    let uploaded = wx_post(
        cred,
        "getuploadurl",
        &json!({
            "filekey":filekey, "media_type":3, "to_user_id":target.chat_id,
            "rawsize":data.len(), "rawfilemd5":format!("{:x}",md5::compute(data)),
            "filesize":encrypted.len(), "no_need_thumb":true, "aeskey":key_hex,
            "base_info":{"channel_version":"0.1.0"}
        }),
    )
    .await?;
    let url = if let Some(full) = uploaded["upload_full_url"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        reqwest::Url::parse(full)?
    } else {
        let mut url = reqwest::Url::parse("https://novac2c.cdn.weixin.qq.com/c2c/upload")?;
        url.query_pairs_mut()
            .append_pair(
                "encrypted_query_param",
                uploaded["upload_param"]
                    .as_str()
                    .context("missing upload parameter")?,
            )
            .append_pair("filekey", &filekey);
        url
    };
    anyhow::ensure!(
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|h| h.ends_with(".weixin.qq.com")),
        "unexpected WeChat CDN URL"
    );
    let response = client()?
        .post(url)
        .header("Content-Type", "application/octet-stream")
        .body(encrypted)
        .send()
        .await?
        .error_for_status()?;
    let remote = response
        .headers()
        .get("X-Encrypted-Param")
        .context("CDN upload missing download parameter")?
        .to_str()?;
    Ok(json!({
        "msg":{"from_user_id":"","to_user_id":target.chat_id,"client_id":id,"message_type":2,"message_state":2,"context_token":target.context_token,
        "item_list":[{"type":4,"file_item":{"file_name":name,"len":data.len().to_string(),
        "media":{"encrypt_query_param":remote,"aes_key":base64::engine::general_purpose::STANDARD.encode(key_hex.as_bytes()),"encrypt_type":1}}}]},
        "base_info":{"channel_version":"0.1.0"}
    }))
}
pub async fn send(
    cred: &WechatCredential,
    target: &ReplyTarget,
    text: &str,
    id: &str,
) -> Result<()> {
    let request = prepare(cred, target, text, id).await?;
    wx_post(cred, "sendmessage", &request).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encryption_matches_ilink_sdk_fixture() {
        let key = std::array::from_fn(|i| i as u8);
        let encrypted = encrypt(b"raw JSON bytes", &key);
        assert_eq!(
            encrypted
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            "d176fb290642ca923eafa1a37bb12ec0"
        );
        assert_eq!(encrypt(&[0; 16], &key).len(), 32);
        assert_eq!(encrypt(&[], &key).len(), 16);
    }
}
