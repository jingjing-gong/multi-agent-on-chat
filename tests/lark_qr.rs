use multi_agent_on_chat::channels;
use std::{os::unix::fs::PermissionsExt, sync::Mutex};

#[tokio::test]
async fn qr_link_is_forwarded_before_registration_finishes() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("lark-cli");
    std::fs::write(&bin, r#"#!/usr/bin/python3
import os,sys,json,pathlib,time
args=sys.argv[1:]
root=pathlib.Path(os.environ['QR_TEST_ROOT'])
if args[:2]==['config','init']:
 assert '--new' in args
 print('  https://accounts.feishu.cn/register?token=a%2Bb&next=%2Fapp',file=sys.stderr,flush=True)
 for i in range(100):
  if (root/'notified').exists(): break
  time.sleep(.02)
 else: sys.exit(3)
elif args[:2]==['auth','qrcode']:
 assert args[2]=='https://accounts.feishu.cn/register?token=a%2Bb&next=%2Fapp'
 pathlib.Path('login-qr.png').write_bytes(b'PNG-test')
elif args==['auth','status','--json','--verify']:
 print(json.dumps({'identity':'user','verified':True,'appId':'cli_test','brand':'feishu','identities':{'bot':{'status':'ready','available':True,'verified':True,'openId':'ou_bot'}}}))
elif args==['whoami','--as','bot']:
 print(json.dumps({'identity':'bot','appId':'cli_test','brand':'feishu'}))
else: sys.exit(4)
"#).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let old = std::env::var_os("PATH").unwrap();
    std::env::set_var("PATH", temp.path());
    std::env::set_var("QR_TEST_ROOT", temp.path());
    let seen = Mutex::new(Vec::new());
    let result = channels::lark_login_with_link(
        &temp.path().join("credentials"),
        None,
        "feishu",
        None,
        |url, image| {
            assert_eq!(std::fs::read(image)?, b"PNG-test");
            seen.lock().unwrap().push(url.to_owned());
            std::fs::write(temp.path().join("notified"), b"ok")?;
            Ok(())
        },
    )
    .await;
    std::env::set_var("PATH", old);
    std::env::remove_var("QR_TEST_ROOT");
    assert_eq!(result.unwrap(), "cli_test");
    assert_eq!(seen.lock().unwrap().len(), 1);
}
