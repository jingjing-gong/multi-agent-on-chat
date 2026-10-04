use multi_agent_on_chat::{domain::Template, engine::CodexEngine};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn catalog_paginates_without_starting_a_thread_and_rejects_bad_pages() {
    for mode in ["pages", "malformed", "repeated"] {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("fake-codex");
        let script = r#"#!/usr/bin/env python3
import json,sys
mode = MODE
for line in sys.stdin:
    message=json.loads(line)
    if 'id' not in message: continue
    method=message['method']
    if method=='initialize': result={}
    elif method=='config/read': result={'config':{'model_provider':'openai'}}
    elif method=='model/list':
        cursor=message['params'].get('cursor')
        assert message['params']['includeHidden'] is False
        if mode=='malformed': result={'data':{}}
        elif mode=='repeated': result={'data':[],'nextCursor':'same'}
        elif cursor is None: result={'data':[{'id':'first'}],'nextCursor':'next'}
        else:
            assert cursor=='next'
            result={'data':[{'id':'second'}],'nextCursor':None}
    else: raise Exception('unexpected method: '+method)
    print(json.dumps({'id':message['id'],'result':result}),flush=True)
"#
        .replace("MODE", &format!("{mode:?}"));
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config: Template =
            serde_json::from_value(json!({"id":"test","model":"configured-only"})).unwrap();
        let result =
            CodexEngine::model_catalog(executable.to_str().unwrap(), &config, dir.path()).await;
        match mode {
            "pages" => assert_eq!(
                result.unwrap(),
                json!({"data":[{"id":"first"},{"id":"second"}],"nextCursor":null})
            ),
            "malformed" => assert!(result.unwrap_err().to_string().contains("data array")),
            _ => assert!(result.unwrap_err().to_string().contains("repeated")),
        }
    }
}
