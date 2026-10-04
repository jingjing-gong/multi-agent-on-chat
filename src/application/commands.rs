//! Shared chat/tool adapter parsing. Free text is never tokenized as options.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
pub fn validate(command: &str, rest: &str) -> Result<()> {
    let words = rest.split_whitespace().collect::<Vec<_>>();
    let max = match command {
        "/clear" | "/compact" | "/status" | "/tasks" | "/collaborations" | "/inbox" | "/recv"
        | "确认" | "confirm" | "取消" | "cancel" => Some(0),
        "/cancel" | "/retry" | "/stream" => Some(1),
        "/cron" if words.first().is_none_or(|s| *s == "list") => Some(1),
        _ => None,
    };
    if max.is_some_and(|max| words.len() > max) {
        bail!(
            "参数过多；使用 /help {} 查看用法",
            command.trim_start_matches('/')
        );
    }
    if command == "/bot"
        && words.first().is_some_and(|s| matches!(*s, "list" | "help"))
        && words.len() > 1
    {
        bail!("参数过多；/bot help 查看用法");
    }
    Ok(())
}
pub fn pagination(text: &str) -> Result<Option<(String, usize, usize)>> {
    let command = text.split_whitespace().next().unwrap_or("");
    let action = text.split_whitespace().nth(1).unwrap_or("");
    if matches!(command, "/cron" | "/template" | "/skills")
        && !matches!(action, "" | "list" | "available" | "--page" | "--limit")
    {
        return Ok(None);
    }
    if !matches!(
        command,
        "/tasks"
            | "/collaborations"
            | "/inbox"
            | "/recv"
            | "/agent"
            | "/bot"
            | "/model"
            | "/profile"
            | "/cron"
            | "/template"
            | "/skills"
    ) {
        return Ok(None);
    }
    if !text
        .split_whitespace()
        .any(|s| matches!(s, "--page" | "--limit"))
    {
        return Ok(None);
    }
    let words = shlex::split(text).context("引号不匹配")?;
    let cmd = words.first().map(String::as_str).unwrap_or("");
    let list = matches!(cmd, "/tasks" | "/collaborations" | "/inbox" | "/recv")
        || (matches!(cmd, "/agent" | "/bot" | "/model" | "/profile")
            && words.get(1).is_some_and(|s| s == "list"))
        || (matches!(cmd, "/cron" | "/template" | "/skills")
            && words
                .get(1)
                .is_none_or(|s| matches!(s.as_str(), "list" | "available" | "--page" | "--limit")));
    if !list
        || !words
            .iter()
            .any(|s| matches!(s.as_str(), "--page" | "--limit"))
    {
        return Ok(None);
    }
    let (mut page, mut limit) = (None, None);
    let mut base = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let slot = match words[i].as_str() {
            "--page" => Some(&mut page),
            "--limit" => Some(&mut limit),
            _ => None,
        };
        if let Some(slot) = slot {
            if slot.is_some() {
                bail!("分页参数不能重复");
            }
            i += 1;
            *slot = Some(
                words
                    .get(i)
                    .context("分页参数缺少数值")?
                    .parse::<usize>()
                    .context("分页参数必须为正整数")?,
            );
        } else {
            base.push(words[i].clone());
        }
        i += 1;
    }
    let (page, limit) = (page.unwrap_or(1), limit.unwrap_or(20));
    anyhow::ensure!(
        page > 0 && (1..=100).contains(&limit) && page.checked_mul(limit).is_some(),
        "page 必须为正数，limit 为 1–100"
    );
    Ok(Some((base.join(" "), page, limit)))
}
pub fn page(mut result: Value, page: usize, limit: usize) -> Result<Value> {
    let key = ["skills", "bots", "data", "profiles"]
        .into_iter()
        .find(|key| result[*key].is_array());
    let rows = if result.is_array() {
        result.as_array_mut().unwrap()
    } else {
        result[key.context("该命令不支持分页")?]
            .as_array_mut()
            .unwrap()
    };
    let total = rows.len();
    let items = rows
        .iter()
        .skip((page - 1) * limit)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    if let Some(key) = key {
        result[key] = json!([]);
    }
    Ok(
        json!({"items":items,"page":page,"limit":limit,"total":total,"has_more":page*limit<total,"result_key":key,"context":if result.is_object(){result}else{json!({})}}),
    )
}
