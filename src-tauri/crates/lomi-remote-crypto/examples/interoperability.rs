#[cfg(feature = "test-fixtures")]
fn main() {
    use lomi_remote_crypto::*;
    use std::io::Read;
    let host = Identity::fixture(Role::Host, 11).unwrap();
    let mut device = Identity::fixture(Role::Device, 21).unwrap();
    let hb = host.public_bundle();
    let db = device.public_bundle();
    let policy = MailboxPolicy {
        account_id: [1; 16],
        sender_host_id: [2; 16],
        recipient_device_id: [3; 16],
        grant_ref: [4; 16],
        access_epoch: 1,
        grant_revision: 1,
        grant_deadline: 2000,
    };
    if std::env::args().nth(1).as_deref() == Some("verify") {
        let mut input = String::new();
        std::io::stdin()
            .take(MAX_JSON as u64 + 1)
            .read_to_string(&mut input)
            .unwrap();
        assert!(input.len() <= MAX_JSON);
        let envelope: Envelope = serde_json::from_str(&input).unwrap();
        let plaintext = device
            .open_mailbox(
                &envelope,
                &hb,
                &hb.bundle.fingerprint().unwrap(),
                &policy,
                1100,
            )
            .unwrap();
        assert_eq!(&*plaintext, b"browser-to-native");
        println!("browser-to-native verified");
        return;
    }
    let envelope = host
        .seal_mailbox(
            &db,
            &db.bundle.fingerprint().unwrap(),
            &policy,
            1000,
            1500,
            b"native-to-browser",
        )
        .unwrap();
    let context = ChannelContext {
        version: 1,
        account_id: [1; 16],
        host_id: [2; 16],
        device_id: [3; 16],
        initiator_role: Role::Device,
        responder_role: Role::Host,
    };
    println!(
        "{}",
        serde_json::json!({"host_bundle":hb,"device_bundle":db,"host_pin":hb.bundle.fingerprint().unwrap(),"device_pin":db.bundle.fingerprint().unwrap(),"host_canonical":hb.bundle.canonical().unwrap(),"device_canonical":db.bundle.canonical().unwrap(),"context":context,"policy":policy,"envelope":envelope})
    );
}
#[cfg(not(feature = "test-fixtures"))]
fn main() {
    eprintln!("requires --features test-fixtures");
    std::process::exit(1);
}
