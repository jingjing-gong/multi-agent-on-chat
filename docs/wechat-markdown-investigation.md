# WeChat iLink Markdown investigation

> 分类：调查记录。客户端行为具有版本和渠道范围；现行输出契约见 [命令输出](command-output.md)。

Inspected the published Tencent package `@tencent-weixin/openclaw-weixin` 2.4.9 on 2026-09-28. Package registry: https://www.npmjs.com/package/@tencent-weixin/openclaw-weixin

Reproducible source archive: https://registry.npmjs.org/@tencent-weixin/openclaw-weixin/-/openclaw-weixin-2.4.9.tgz

- `CHANGELOG.zh_CN.md`, 2.1.3 (2026-04-02): explicitly says Markdown changed from unsupported to partially supported; removed `markdownToPlainText` in favor of `StreamingMarkdownFilter`. Version 2.1.8 (2026-04-07) preserves additional formats.
- `src/messaging/markdown-filter.ts`: documents passthrough for fences, inline code, tables, horizontal rules, bold, non-CJK italic/bold-italic. Filters CJK italic, H5/H6 and Markdown images.
- `src/messaging/send.ts`: `buildSendMessageReq` places text inside `MessageItemType.TEXT` / `text_item.text`; a separate Markdown message type is not required.
- `src/api/types.ts`: supported items include text/image/voice/file/video/tool events. No dedicated HTML item or HTML rendering capability was found. Sending HTML characters as text or an HTML attachment does not demonstrate native HTML rendering.

Correction: the earlier inference that TEXT-only means no Markdown rendering was wrong. The WeChat adapter now preserves Markdown and converts generic rich-text emphasis to Markdown. Common message/presentation modules are unchanged. This implementation passes Markdown through without claiming full CommonMark support; client/version-dependent behavior still requires visual verification on the user's device. No real message was sent during this investigation.
