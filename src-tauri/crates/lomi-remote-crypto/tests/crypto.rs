use lomi_remote_crypto::*;
fn identities() -> (Identity, Identity, ChannelContext) {
    let account = [1; 16];
    (
        Identity::generate(account, [2; 16], Role::Host, 1).unwrap(),
        Identity::generate(account, [3; 16], Role::Device, 1).unwrap(),
        ChannelContext {
            version: 1,
            account_id: account,
            host_id: [2; 16],
            device_id: [3; 16],
            initiator_role: Role::Device,
            responder_role: Role::Host,
        },
    )
}
fn handshake(host: &Identity, device: &Identity, context: &ChannelContext) -> (Channel, Channel) {
    let hb = host.public_bundle();
    let db = device.public_bundle();
    let mut client = Handshake::new(
        true,
        device,
        context,
        &hb,
        &hb.bundle.fingerprint().unwrap(),
    )
    .unwrap();
    let mut server =
        Handshake::new(false, host, context, &db, &db.bundle.fingerprint().unwrap()).unwrap();
    server.read(&client.write().unwrap()).unwrap();
    client.read(&server.write().unwrap()).unwrap();
    server.read(&client.write().unwrap()).unwrap();
    (server.finish().unwrap(), client.finish().unwrap())
}
fn policy() -> MailboxPolicy {
    MailboxPolicy {
        account_id: [1; 16],
        sender_host_id: [2; 16],
        recipient_device_id: [3; 16],
        grant_ref: [4; 16],
        access_epoch: 1,
        grant_revision: 1,
        grant_deadline: 2000,
    }
}
#[test]
fn complete_bundle_signature_binds_every_key_and_role() {
    let (host, _, _) = identities();
    let bundle = host.public_bundle();
    let pin = bundle.bundle.fingerprint().unwrap();
    bundle.verify(&pin).unwrap();
    for which in 0..4 {
        let mut changed = bundle.clone();
        match which {
            0 => changed.bundle.channel_key[0] ^= 1,
            1 => changed.bundle.mailbox_key[0] ^= 1,
            2 => changed.bundle.identity_key[0] ^= 1,
            _ => changed.bundle.role = Role::Device,
        };
        assert!(changed.verify(&pin).is_err());
        let forged_pin = changed.bundle.fingerprint().unwrap();
        assert!(changed.verify(&forged_pin).is_err());
    }
}
#[test]
fn noise_roundtrip_transcript_binding_and_replay_rejection() {
    let (host, device, context) = identities();
    let (mut server, mut client) = handshake(&host, &device, &context);
    assert_eq!(server.transcript_hash, client.transcript_hash);
    let frame = client.seal([9; 16], "界e\u{301}".as_bytes()).unwrap();
    assert_eq!(
        &*server.open([9; 16], &frame).unwrap(),
        "界e\u{301}".as_bytes()
    );
    assert!(server.open([9; 16], &frame).is_err());
    assert!(server.seal([9; 16], b"closed").is_err());
}
#[test]
fn wrong_pin_context_route_and_tamper_fail_closed() {
    let (host, device, context) = identities();
    let hb = host.public_bundle();
    assert!(Handshake::new(true, &device, &context, &hb, &[0; 32]).is_err());
    let mut bad = context.clone();
    bad.account_id = [8; 16];
    assert!(Handshake::new(true, &device, &bad, &hb, &hb.bundle.fingerprint().unwrap()).is_err());
    let (mut server, mut client) = handshake(&host, &device, &context);
    let mut frame = client.seal([9; 16], b"secret").unwrap();
    frame.ciphertext[0] ^= 1;
    assert!(server.open([9; 16], &frame).is_err());
    let (mut server, mut client) = handshake(&host, &device, &context);
    let mut frame = client.seal([9; 16], b"secret").unwrap();
    frame.routing = [7; 16];
    assert!(server.open([7; 16], &frame).is_err());
}
#[test]
fn valid_signed_static_substitution_cannot_enter_transport() {
    let (host, device, context) = identities();
    let impostor = Identity::generate([1; 16], [2; 16], Role::Host, 1).unwrap();
    let hb = host.public_bundle();
    let db = device.public_bundle();
    let mut client = Handshake::new(
        true,
        &device,
        &context,
        &hb,
        &hb.bundle.fingerprint().unwrap(),
    )
    .unwrap();
    let mut rogue = Handshake::new(
        false,
        &impostor,
        &context,
        &db,
        &db.bundle.fingerprint().unwrap(),
    )
    .unwrap();
    rogue.read(&client.write().unwrap()).unwrap();
    assert!(client.read(&rogue.write().unwrap()).is_err());
    assert!(client.finish().is_err());
}
#[test]
fn mailbox_sender_recipient_policy_signature_expiry_and_replay() {
    let (host, mut device, _) = identities();
    let hb = host.public_bundle();
    let db = device.public_bundle();
    let hp = hb.bundle.fingerprint().unwrap();
    let dp = db.bundle.fingerprint().unwrap();
    let policy = policy();
    let envelope = host
        .seal_mailbox(&db, &dp, &policy, 1000, 1500, b"completed")
        .unwrap();
    let mut changed = envelope.clone();
    changed.enc[0] ^= 1;
    assert!(device
        .open_mailbox(&changed, &hb, &hp, &policy, 1100)
        .is_err());
    let mut changed = envelope.clone();
    changed.ciphertext[0] ^= 1;
    assert!(device
        .open_mailbox(&changed, &hb, &hp, &policy, 1100)
        .is_err());
    let mut changed = envelope.clone();
    changed.metadata.recipient_device_id = [9; 16];
    assert!(device
        .open_mailbox(&changed, &hb, &hp, &policy, 1100)
        .is_err());
    let mut changed_policy = policy.clone();
    changed_policy.access_epoch = 2;
    assert!(device
        .open_mailbox(&envelope, &hb, &hp, &changed_policy, 1100)
        .is_err());
    assert!(device
        .open_mailbox(&envelope, &hb, &hp, &policy, 1500)
        .is_err());
    assert_eq!(
        &*device
            .open_mailbox(&envelope, &hb, &hp, &policy, 1100)
            .unwrap(),
        b"completed"
    );
    assert!(device
        .open_mailbox(&envelope, &hb, &hp, &policy, 1101)
        .unwrap_err()
        .contains("replay"));
    assert!(host
        .seal_mailbox(&db, &dp, &policy, 1000, 2001, b"past grant")
        .is_err());
}
#[test]
fn caps_and_wrong_key_reject() {
    let (host, mut device, _) = identities();
    let hb = host.public_bundle();
    let db = device.public_bundle();
    let policy = policy();
    assert!(host
        .seal_mailbox(
            &db,
            &db.bundle.fingerprint().unwrap(),
            &policy,
            1000,
            1500,
            &vec![0; MAX_MAILBOX_PAYLOAD + 1]
        )
        .is_err());
    let other = Identity::generate([1; 16], [3; 16], Role::Device, 2).unwrap();
    let ob = other.public_bundle();
    let envelope = host
        .seal_mailbox(
            &ob,
            &ob.bundle.fingerprint().unwrap(),
            &policy,
            1000,
            1500,
            b"future key",
        )
        .unwrap();
    assert!(device
        .open_mailbox(
            &envelope,
            &hb,
            &hb.bundle.fingerprint().unwrap(),
            &policy,
            1100
        )
        .is_err());
}

#[test]
fn malformed_and_oversized_noise_inputs_cannot_recover_channel() {
    let (host, device, context) = identities();
    let hb = host.public_bundle();
    let db = device.public_bundle();
    let mut server = Handshake::new(
        false,
        &host,
        &context,
        &db,
        &db.bundle.fingerprint().unwrap(),
    )
    .unwrap();
    assert!(server.read(&vec![0; 513]).is_err());
    assert!(server.write().is_err());
    assert!(server.finish().is_err());
    let mut client = Handshake::new(
        true,
        &device,
        &context,
        &hb,
        &hb.bundle.fingerprint().unwrap(),
    )
    .unwrap();
    assert!(client.read(b"bad").is_err());
    assert!(client.write().is_err());
    let (mut server, mut client) = handshake(&host, &device, &context);
    assert!(client.seal([9; 16], &vec![0; MAX_PLAINTEXT + 1]).is_err());
    let frame = client.seal([9; 16], &vec![1; MAX_PLAINTEXT]).unwrap();
    assert_eq!(server.open([9; 16], &frame).unwrap().len(), MAX_PLAINTEXT);
}
#[test]
fn canonical_arrays_are_versioned_and_have_exact_arity() {
    let (host, _, context) = identities();
    let public = host.public_bundle().bundle.canonical().unwrap();
    let mut d = minicbor::Decoder::new(&public);
    assert_eq!(d.array().unwrap(), Some(10));
    for _ in 0..10 {
        d.skip().unwrap();
    }
    assert_eq!(d.position(), public.len());
    let canonical = context.canonical().unwrap();
    let mut d = minicbor::Decoder::new(&canonical);
    assert_eq!(d.array().unwrap(), Some(8));
    for _ in 0..8 {
        d.skip().unwrap();
    }
    assert_eq!(d.position(), canonical.len());
}
#[cfg(feature = "test-fixtures")]
#[test]
fn fixed_bundle_fingerprints_are_stable_native_wasm_fixtures() {
    let hex = |bytes: [u8; 32]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    assert_eq!(
        hex(Identity::fixture(Role::Host, 11)
            .unwrap()
            .public_bundle()
            .bundle
            .fingerprint()
            .unwrap()),
        "ee8d7bc0a65545476f3f6c7689f42f35ee91825bdb9b9e06bc83b0b878cfea52"
    );
    assert_eq!(
        hex(Identity::fixture(Role::Device, 21)
            .unwrap()
            .public_bundle()
            .bundle
            .fingerprint()
            .unwrap()),
        "31e5e3ba75d4f5368a980ccc8b553615ebe75ecabdf155df7c44b43541858ce8"
    );
}
