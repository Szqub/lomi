// Explicit test-fixtures feature is required; these seeds never enter default builds.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { chromium } from '@playwright/test';
const crate = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = resolve(crate, 'Cargo.toml');
const fixture = JSON.parse(execFileSync('cargo', ['run', '--quiet', '--locked', '--manifest-path', manifest, '--features', 'test-fixtures', '--example', 'interoperability'], {encoding:'utf8'}));
const server = createServer((req,res) => {
  if(req.url==='/fixture') {res.setHeader('Content-Type','application/json');res.end(JSON.stringify(fixture));return;}
  if(req.url==='/') {res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Unqualified crypto test</title>');return;}
  const files = {'/crypto.js':['lomi_remote_crypto.js','text/javascript'],'/lomi_remote_crypto_bg.wasm':['lomi_remote_crypto_bg.wasm','application/wasm']};
  const entry=files[req.url];if(!entry){res.writeHead(404);res.end();return;}
  res.setHeader('Content-Type',entry[1]);res.end(readFileSync(resolve(crate,'pkg',entry[0])));
});
await new Promise(resolve => server.listen(0,'127.0.0.1',resolve));
let browser;
try {
  browser=await chromium.launch({headless:true});
  const page=await browser.newPage();await page.goto(`http://127.0.0.1:${server.address().port}`);
  const result=await page.evaluate(async()=>{
    const crypto=await import('/crypto.js');await crypto.default();const f=await(await fetch('/fixture')).json();
    const fail=(condition,message)=>{if(!condition)throw Error(message);};
    const equal=(a,b)=>JSON.stringify(Array.from(a))===JSON.stringify(Array.from(b));
    const rejects=(fn)=>{try{fn();return false;}catch{return true;}};
    fail(crypto.production_qualified()===false,'production gate');
    const host=crypto.BrowserIdentity.fixture(true,11),device=crypto.BrowserIdentity.fixture(false,21);
    fail(equal(host.fingerprint(),f.host_pin),'native/WASM host fingerprint');fail(equal(device.fingerprint(),f.device_pin),'native/WASM device fingerprint');
    crypto.verify_public_bundle(JSON.stringify(f.host_bundle),new Uint8Array(f.host_pin));
    const changed=structuredClone(f.host_bundle);changed.bundle.mailbox_key[0]^=1;fail(rejects(()=>crypto.verify_public_bundle(JSON.stringify(changed),new Uint8Array(f.host_pin))),'substituted mailbox key');
    const context=JSON.stringify(f.context),hb=host.public_bundle(),db=device.public_bundle();
    const client=new crypto.BrowserHandshake(true,device,context,hb,host.fingerprint());const server=new crypto.BrowserHandshake(false,host,context,db,device.fingerprint());
    server.read(client.write());client.read(server.write());server.read(client.write());
    const c=client.finish(),s=server.finish();fail(equal(c.transcript_hash(),s.transcript_hash()),'Noise transcript');
    const routing=new Uint8Array(16).fill(9),payload=new TextEncoder().encode('界e\u0301');const frame=c.seal(routing,payload);fail(equal(s.open(routing,frame),payload),'WASM Noise roundtrip');fail(rejects(()=>s.open(routing,frame)),'Noise replay');
    const policy=JSON.stringify(f.policy),envelope=JSON.stringify(f.envelope),hostPin=new Uint8Array(f.host_pin);
    fail(new TextDecoder().decode(device.open_mailbox(envelope,hb,hostPin,policy,1100n))==='native-to-browser','native HPKE to browser');
    fail(rejects(()=>device.open_mailbox(envelope,hb,hostPin,policy,1100n)),'HPKE replay');
    const tampered=structuredClone(f.envelope);tampered.enc[0]^=1;fail(rejects(()=>device.open_mailbox(JSON.stringify(tampered),hb,hostPin,policy,1100n)),'HPKE enc tamper');
    fail(rejects(()=>device.open_mailbox(envelope,hb,hostPin,policy,1500n)),'HPKE expiry');
    const ephemeral=new crypto.BrowserIdentity(new Uint8Array(16).fill(1),new Uint8Array(16).fill(3),false,1);fail(ephemeral.fingerprint().length===32,'browser entropy generation');ephemeral.free();
    const outgoing=host.seal_mailbox(db,device.fingerprint(),policy,1000n,1500n,new TextEncoder().encode('browser-to-native'));
    host.free();device.free();c.free();s.free();return {outgoing,checks:11};
  });
  const verification=execFileSync('cargo',['run','--quiet','--locked','--manifest-path',manifest,'--features','test-fixtures','--example','interoperability','--','verify'],{input:result.outgoing,encoding:'utf8'});
  assert.match(verification,/browser-to-native verified/);console.log(`Chromium WASM: ${result.checks} checks; bidirectional native/WASM HPKE verified`);
} finally {await browser?.close();await new Promise(resolve=>server.close(resolve));}
