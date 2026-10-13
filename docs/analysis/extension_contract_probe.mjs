// Executes the original extension source against controlled browser API mocks.
// This is component evidence, not real-browser or OS-integration evidence.
import fs from 'node:fs';
import vm from 'node:vm';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';

const [sourceDirectory, outputDirectory] = process.argv.slice(2);
if (!sourceDirectory || !outputDirectory) throw new Error('Usage: node extension_contract_probe.mjs SOURCE_DIR OUTPUT_DIR');
fs.mkdirSync(outputDirectory, { recursive: true });
const source = fs.readFileSync(`${sourceDirectory}/background.js`, 'utf8');
const manifest = JSON.parse(fs.readFileSync(`${sourceDirectory}/manifest.json`, 'utf8'));
const calls = [];
const outbound = [];
const pageMessages = [];
const events = new Map();
const event = name => {
  if (!events.has(name)) events.set(name, { listeners: [], addListener(fn, ...args) { this.listeners.push(fn); calls.push({api:name, operation:'addListener', args}); }, removeListener(fn) { this.listeners = this.listeners.filter(x => x !== fn); } });
  return events.get(name);
};
const method = (name, result) => (...args) => {
  calls.push({api:name, args:args.filter(x => typeof x !== 'function')});
  const callback = args.find(x => typeof x === 'function');
  if (callback) queueMicrotask(() => callback(result));
  return Promise.resolve(result);
};
const namespace = name => new Proxy({}, {get(target, prop) {
  if (prop in target) return target[prop];
  if (String(prop).startsWith('on')) return event(`${name}.${String(prop)}`);
  return target[prop] = method(`${name}.${String(prop)}`, undefined);
}});
const nativePort = {
  onDisconnect:event('nativePort.onDisconnect'), onMessage:event('nativePort.onMessage'),
  postMessage(message) { outbound.push(message); }, disconnect() {},
};
const chrome = {
  runtime:namespace('runtime'), action:namespace('action'), contextMenus:namespace('contextMenus'),
  windows:namespace('windows'), tabs:namespace('tabs'), downloads:namespace('downloads'),
  webNavigation:namespace('webNavigation'), webRequest:namespace('webRequest'),
  declarativeNetRequest:namespace('declarativeNetRequest'), scripting:namespace('scripting'),
  proxy:{settings:namespace('proxy.settings')}, management:namespace('management'),
  cookies:{getAll:method('cookies.getAll', [{name:'audit_cookie', value:'local_fixture'}]), set:method('cookies.set', {})},
  storage:{local:{get:method('storage.local.get', {version:manifest.version,client:1}),set:method('storage.local.set', {}),remove:method('storage.local.remove', {})}},
  extension:{isAllowedIncognitoAccess:method('extension.isAllowedIncognitoAccess', true)},
  i18n:{getUILanguage:()=> 'en-US',getMessage:key=>key},
};
chrome.runtime.id = 'ngpampappnmepgilojfohadhhmbhlaek';
chrome.runtime.getURL = path => `chrome-extension://${chrome.runtime.id}/${path}`;
chrome.runtime.getManifest = () => manifest;
chrome.runtime.getPlatformInfo = method('runtime.getPlatformInfo', {os:'win'});
chrome.runtime.connectNative = name => {calls.push({api:'runtime.connectNative',name});return nativePort;};
chrome.windows.getAll = method('windows.getAll', []);
chrome.tabs.query = method('tabs.query', []);
chrome.management.get = method('management.get', null);
chrome.proxy.settings.get = method('proxy.settings.get', {value:{mode:'direct'}});
chrome.webRequest.ResourceType = {};
chrome.declarativeNetRequest.getSessionRules = method('declarativeNetRequest.getSessionRules', []);

const timers = new Set();
const context = vm.createContext({
  chrome, browser:chrome, navigator:{userAgent:'Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36'},
  location:{href:chrome.runtime.getURL('background.js')}, screen:{availWidth:1280,availHeight:900},
  URL,Blob,TextEncoder,TextDecoder,Uint8Array,ArrayBuffer,queueMicrotask,
  btoa:s=>Buffer.from(s,'binary').toString('base64'),atob:s=>Buffer.from(s,'base64').toString('binary'),
  setTimeout(fn, delay, ...args) {const id=setTimeout(()=>{timers.delete(id);fn(...args);},delay);timers.add(id);return id;},
  clearTimeout(id){timers.delete(id);clearTimeout(id);},
  WebSocket:class WebSocket { constructor(){this.readyState=3;} close(){} },
  auditCapturePageMessage(message) { pageMessages.push(message); },
});
const boot = vm.runInContext(source, context, {filename:'original/background.js'});
// Observe constructor creation without rewriting the original source file.
vm.runInContext('const AuditOriginalQ=Q; Q=new Proxy(AuditOriginalQ,{construct(target,args){const result=Reflect.construct(target,args);globalThis.auditController=result;return result;}});', context);
await boot;
await new Promise(resolve => setTimeout(resolve, 40));
assert(context.auditController, 'Original extension controller did not initialize');
const initializationMessages=[...outbound];
// Isolate producer/acknowledgment logic from an actual host negotiation.
vm.runInContext('auditController.H=2; auditController.P=113; auditController.S=undefined;', context);
context.auditController.S = nativePort;
outbound.length = 0;
const results = [];

await context.auditController.pa('https://fixture.invalid/media.mp4', 'https://fixture.invalid/player', 'https://fixture.invalid/player', 4, 'audit.mp4', 'local fixture');
assert(outbound.at(-1).startsWith('MSG#') && outbound.at(-1).includes('#14#1#'));
assert(outbound.at(-1).includes('audit_cookie=local_fixture'));
results.push({case:'context_link_payload', result:'passed', messages:[...outbound]});

outbound.length = 0;
const item = {id:701,url:'https://fixture.invalid/original.bin',finalUrl:'https://fixture.invalid/final.bin',filename:'audit.bin',totalBytes:4096,mime:'application/octet-stream',referrer:'https://fixture.invalid/page',startTime:new Date().toISOString()};
context.auditItemSpec = item;
context.auditSuggest = () => calls.push({api:'filename.suggest'});
vm.runInContext('auditController.j[-6]=true; auditController.j[1]=/BIN/; auditController.j[9]=/(?!)/; auditController.j[19]=/(?!)/;', context);
async function prepareDownload(id) {
  context.auditItemSpec = {...item,id};
  vm.runInContext('globalThis.auditDownload=JSON.parse(JSON.stringify(auditItemSpec)); auditController.qb(auditDownload); auditController.rb(auditDownload,auditSuggest);', context);
  await new Promise(resolve => setTimeout(resolve, 10));
  const message = outbound.at(-1);
  assert(message.includes('#13#1#129:'));
  return {message,sequence:Number(/^MSG#(\d+)/.exec(message)[1])};
}
let prepared = await prepareDownload(701);
assert(prepared.message.includes(',56='), 'Redirect cookie context missing');
context.auditReply = {type:1,A:2,j:2,g:[prepared.sequence,3]};
vm.runInContext('auditController.$a(auditReply)', context);
assert(calls.some(x=>x.api==='downloads.cancel' && x.args[0]===701));
assert(!calls.some(x=>x.api==='downloads.resume' && x.args[0]===701));
results.push({case:'accepted_browser_handoff',result:'passed',message:prepared.message});

prepared = await prepareDownload(702);
context.auditReply = {type:1,A:2,j:2,g:[prepared.sequence,0]};
vm.runInContext('auditController.$a(auditReply)', context);
assert(calls.some(x=>x.api==='downloads.resume' && x.args[0]===702));
assert(!calls.some(x=>x.api==='downloads.cancel' && x.args[0]===702));
assert(calls.filter(x=>x.api==='filename.suggest').length===1);
results.push({case:'rejected_browser_handoff',result:'passed',message:prepared.message});

context.auditItemSpec = {...item,id:703};
vm.runInContext('auditController.H=0; globalThis.auditDownload=JSON.parse(JSON.stringify(auditItemSpec)); auditController.qb(auditDownload); auditController.rb(auditDownload,auditSuggest);', context);
await new Promise(resolve=>setTimeout(resolve,10));
assert(calls.some(x=>x.api==='downloads.resume' && x.args[0]===703));
assert(calls.filter(x=>x.api==='filename.suggest').length===2);
results.push({case:'disconnected_send_fallback',result:'passed'});

vm.runInContext('auditController.H=2; auditController.S=undefined;', context);
context.auditController.S = nativePort;
outbound.length=0;
await context.auditController.pa('https://fixture.invalid/media.mp4','https://fixture.invalid/player',null,4,'آزمائش – résumé 🎞.mp4');
results.push({case:'unicode_payload',result:'captured',messages:[...outbound]});

// Decode independently in UTF-8 bytes to check the producer's declared lengths.
function decode(message) {
  const bytes=Buffer.from(message,'utf8');
  const prefix=/^MSG#(\d+)#(\d+)#(\d+)#(\d+)/.exec(message);
  assert(prefix);
  let offset=Buffer.byteLength(prefix[0]);
  const result={sequence:Number(prefix[1]),type:Number(prefix[2]),variant:Number(prefix[3]),flags:Number(prefix[4]),args:[],fields:{}};
  if(bytes[offset]===58){
    const end=[bytes.indexOf(44,offset),bytes.indexOf(59,offset)].filter(x=>x>=0).sort((a,b)=>a-b)[0];
    result.args=bytes.subarray(offset+1,end).toString('ascii').split(':').map(Number);offset=end;
    assert(result.args.every(Number.isSafeInteger), 'Invalid numeric argument');
  }
  while(bytes[offset]===44){
    offset++;
    const equal=bytes.indexOf(61,offset);assert(equal>offset);
    const key=bytes.subarray(offset,equal).toString('ascii');offset=equal+1;
    let end=offset;while(end<bytes.length && bytes[end]>=48 && bytes[end]<=57)end++;
    const number=Number(bytes.subarray(offset,end).toString('ascii'));
    assert(end>offset && Number.isSafeInteger(number), 'Invalid field length/value');
    if(bytes[end]===58){offset=end+1;result.fields[key]=bytes.subarray(offset,offset+number).toString('utf8');offset+=number;}
    else {result.fields[key]=number;offset=end;}
    assert([44,59].includes(bytes[offset]),`Invalid field boundary at ${offset}`);
  }
  assert(bytes[offset]===59 && offset===bytes.length-1);
  return result;
}
assert(decode(outbound.at(-1)).fields['100']==='آزمائش – résumé 🎞.mp4');
results.at(-1).result='passed';

// Install original listeners using explicit test configuration, then feed events.
vm.runInContext(`
auditController.ka=false; auditController.j[-8]=true; auditController.j[-9]=true;
auditController.j[2]=/(?!)/; auditController.j[3]=/^(MP4|WEBM|MP3|M4A)$/;
auditController.j[4]=/(?!)/; auditController.j[10]=/(?!)/;
auditController.j[18]=/(?!)/; auditController.j[20]=/(?!)/; auditController.j[21]=/(?!)/;
auditController.g['1,0']={id:1,tabId:1,G:'https://fixture.invalid/player',postMessage:auditCapturePageMessage};
Bb(auditController);
`, context);
const registered=[...events].filter(([,e])=>e.listeners.length).map(([name])=>name);
assert(registered.includes('downloads.onCreated'));
assert(registered.includes('downloads.onDeterminingFilename'));
assert(registered.includes('webRequest.onBeforeSendHeaders'));
assert(registered.includes('webRequest.onHeadersReceived'));
results.push({case:'event_hook_registration',result:'passed',registered});

for(const [index,path,mime,blocked=false] of [[1,'/movie.mp4','video/mp4'],[2,'/opaque?id=123','video/webm'],[3,'/media.m3u8','application/vnd.apple.mpegurl'],[4,'/media.mpd','application/dash+xml'],[5,'/audio.mp3','audio/mpeg'],[6,'/opaque-blocked?id=123','video/webm',true]]) {
  outbound.length=0;
  vm.runInContext(blocked?'auditController.j[10]=/^video\\/webm$/':'auditController.j[10]=/(?!)/',context);
  context.auditRequestSpec={requestId:String(800+index),tabId:1,frameId:0,type:'media',method:'GET',url:`https://fixture.invalid${path}`,timeStamp:Date.now()};
  context.auditResponseSpec={...context.auditRequestSpec,statusCode:200,statusLine:'HTTP/1.1 200 OK',responseHeaders:[{name:'Content-Type',value:mime},{name:'Content-Length',value:'100000'}],requestHeaders:[{name:'Cookie',value:'audit_cookie=local_fixture'},{name:'User-Agent',value:'Audit-UA/1'},{name:'Referer',value:'https://fixture.invalid/player'}]};
  vm.runInContext(`{const request=JSON.parse(JSON.stringify(auditRequestSpec));const response=JSON.parse(JSON.stringify(auditResponseSpec));auditController.cb(request);auditController.Rb(response);auditController.Ub(response);auditController.Vb(response);}`, context);
  if(blocked){
    assert(outbound.length===0,'Configured MIME exclusion did not suppress inference');
    context.auditRequestId=context.auditRequestSpec.requestId;
    const state=vm.runInContext(`{const r=auditController.i[auditRequestId];r?{action:r.action,is_media:r.h,suppressed:r.g,extension:r.P,inferred_extension:r.na,status:r.statusCode,mime:Z(r.o,'Content-Type'),port:!!r.port}:null}`,context);
    results.push({case:`media_detection_${index}`,path,mime,result:'passed',expected:'suppressed by configured MIME policy',state});
    continue;
  }
  assert(outbound.length>0,`No detection message for ${mime}`);
  const messages=outbound.map(decode);
  assert(messages.some(m=>m.type===13 && m.fields['6']===context.auditRequestSpec.url));
  assert(messages.some(m=>m.fields['11']?.includes('audit_cookie=local_fixture')));
  results.push({case:`media_detection_${index}`,path,mime,result:'passed',messages});
  if(index===3 || index===4){
    const detection=messages.find(m=>m.type===13);
    context.auditReply={type:1,A:2,j:2,g:[detection.sequence,1]};
    vm.runInContext('auditController.$a(auditReply)',context);
    const pageMessage=pageMessages.at(-1);
    assert(pageMessage?.[0]===19 && pageMessage[1]===context.auditRequestSpec.requestId);
    const body=index===3?'#EXTM3U\n#EXTINF:2,\nsegment.ts\n#EXT-X-ENDLIST\n':'<MPD><Period/></MPD>';
    context.auditBody=body;
    context.auditRequestId=context.auditRequestSpec.requestId;
    vm.runInContext('auditController.ub(null,auditRequestId,auditBody)',context);
    const bodyMessage=decode(outbound.at(-1));
    assert(bodyMessage.type===16 && bodyMessage.fields['115']===body);
    results.push({case:`manifest_body_roundtrip_${index}`,result:'passed',pageMessage,bodyMessage});
  }
}

outbound.length=0;
context.auditContextMenu={menuItemId:'1',srcUrl:'blob:https://fixture.invalid/opaque-player',mediaType:'video',pageUrl:'https://fixture.invalid/player',frameId:0};
context.auditTab={id:1};
vm.runInContext('auditController.Ea(auditContextMenu,auditTab)',context);
assert(outbound.length===0);
results.push({case:'blob_context_link',result:'passed',expected:'The blob URI itself is not submitted as a downloadable HTTP resource.'});

// Isolate handoff from request records created by the media cases above.
vm.runInContext('auditController.i={}',context);
prepared=await prepareDownload(704);
const releasesBefore=calls.filter(x=>x.api==='filename.suggest').length;
context.auditDisconnect={error:{message:'controlled native host disconnect'}};
vm.runInContext('auditController.oa(auditDisconnect)',context);
await new Promise(resolve=>setTimeout(resolve,30));
const pending=!!context.auditController.ha[704];
const resumed=calls.some(x=>x.api==='downloads.resume' && x.args[0]===704);
results.push({case:'disconnect_with_pending_handoff',result:'bounded_observation',observation_window_ms:30,pending_entry:pending,browser_resume_called:resumed,new_filename_callback_calls:calls.filter(x=>x.api==='filename.suggest').length-releasesBefore});

for(const timer of timers)clearTimeout(timer);
const evidence={scope:'Original extension source in a VM with mocked browser APIs and simulated host replies. No actual browser/native-host integration claim.',source_sha256:crypto.createHash('sha256').update(source).digest('hex'),manifest_version:manifest.version,initializationMessages,results,calls};
fs.writeFileSync(`${outputDirectory}/extension_contract_results.json`,JSON.stringify(evidence,null,2)+'\n');
console.log(JSON.stringify({cases:results.length,passed:results.filter(x=>x.result==='passed').length,output:`${outputDirectory}/extension_contract_results.json`}));
