"""Individual resource-control semantic inventory; static evidence is not runtime parity."""
import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'docs/analysis/idm_ui_resources.json'
LANG = ROOT / 'docs/analysis/idm_language_catalog.json'
REVIEW = ROOT / 'docs/idm_behavior_review.json'
# Manually reviewed bindings for blank inputs/collections and generic resource names.
# Keys are resource dialog ID then zero-based control index, never globally reused control IDs.
BINDINGS = {
100: {8:'Read-only about/license detail text',14:'Additional license information',15:'Additional license information'},
102: {0:'Main download task collection',1:'Category and queue navigation tree',4:'Close category navigation'},
103: {0:'Overall download progress',5:'Per-connection start offsets and progress collection',7:'Download status detail surface',8:'Download status layout surface'},
105: {2:'Current tip text'},
130: {4:'Download URL',6:'Destination category',7:'Create destination category',9:'Output path/history',10:'Choose output path',13:'Output filename within remembered category path',14:'Task description',16:'File information output'},
135: {1:'Filename summary',4:'File type value',6:'Task status value',8:'File size value',10:'Output path',11:'Choose output path',13:'Source URL',15:'Task description',18:'Originating web page value',21:'Referer URL',23:'HTTP username',25:'HTTP password',28:'Last attempt/result output'},
138: {3:'Registration first name',5:'Registration last name',7:'Registration email',9:'Registration serial number'},
139: {7:'Integrated browser selection collection',11:'Configure browser capture modifier keys',13:'Configure browser context menu entries',14:'Configure browser download panels'},
140: {12:'Automatic proxy configuration script URL',18:'Proxy server address',19:'Proxy server port',20:'Proxy username',21:'Proxy password',28:'Proxy bypass address prefixes'},
141: {1:'Per-server connection limit exception collection',7:'Download quota in MBytes',10:'Quota interval in hours',14:'Default maximum connections'},
143: {6:'Category selection',8:'Category file extension patterns',10:'Category destination directory',11:'Choose category directory',15:'Temporary partial-file directory',16:'Choose temporary directory'},
144: {2:'Update release details'},145:{3:'Site credential profile collection'},
146: {0:'Site protocol choice',2:'Server/path credential scope',5:'Site username',7:'Site password'},
147: {4:'Dynamic confirmation message'},148:{1:'Download URL/history',5:'HTTP username',7:'HTTP password'},
150: {1:'Shutdown countdown progress'},182:{0:'Update check progress'},
185: {3:'Windows dial-up/VPN connection selection',5:'Connection username',7:'Connection password',12:'Redial attempt count (zero indicates endless)',15:'Redial interval in seconds'},
186: {1:'Connection username',3:'Connection password'},191:{},193:{0:'Automatically captured extension collection'},
194: {1:'Quota warning message'},195:{4:'Event sound configuration collection',5:'Choose event sound file',6:'Preview event sound'},196:{1:'ZIP archive member collection'},
204: {1:'Category name',3:'Category extension patterns',7:'Category site wildcard patterns',11:'Category destination folder',13:'Choose category folder'},
208: {1:'Interface language selection',3:'Language information'},209:{1:'Column visibility/order selection collection',8:'Selected column width in pixels'},
226: {1:'Server protocol selection',2:'Server wildcard pattern',5:'Maximum connections for matching server'},
228: {1:'Discovered page link selection collection',6:'Single destination category',7:'Single destination directory',8:'Choose single directory',12:'Hide repeated files',13:'Replacement filename wildcard pattern'},
231: {8:'Integrated browser selection collection',12:'Configure browser capture modifier keys',14:'Configure browser context menu entries',15:'Configure browser download panels'},
232: {1:'Generated configuration report',3:'User report comments',5:'Notification email address'},
236: {2:'Automatic link capture guidance text; exact runtime text producer remains unverified'},
240: {0:'Included file type filter',2:'Excluded file type filter',12:'Minimum allowed file size',13:'Minimum file size unit',15:'Maximum allowed file size',16:'Maximum file size unit',18:'Allowed file path/domain patterns',20:'Excluded file path/domain patterns'},
241: {0:'Grabber explored page hierarchy',1:'Discovered grabber file collection',6:'Dynamic grabber status indicator',7:'Dynamic grabber status indicator',8:'Dynamic grabber status indicator',9:'Dynamic grabber status indicator',10:'Dynamic grabber status indicator'},
244: {0:'Named file filter collection'},245:{1:'Filter name',3:'Comma-separated wildcard masks'},
257: {1:'Grabber project name',3:'Grabber seed URL',5:'Grabber project template choice',10:'Grabber HTTP username',12:'Grabber HTTP password',16:'Manual login page URL',18:'Logout page exclusion wildcard'},
259: {3:'Grabber destination category',4:'Resolved category folder',6:'Grabber single destination folder',8:'Choose grabber destination folder'},
262: {0:'Parallel grabber exploration files (1 to 10)',2:'Parallel grabber download files (1 to 10)'},
264: {3:'Crawl depth within base site',5:'Crawl depth on other sites',15:'Allowed exploration path/domain patterns',17:'Excluded exploration path/domain patterns'},
268: {0:'Grabber template name'},269:{1:'Saved grabber project name'},270:{2:'Saved grabber project selection'},
271: {1:'Total explored progress value',12:'Dynamic grabber statistic value; metric binding not runtime verified',13:'Dynamic grabber statistic value; metric binding not runtime verified',14:'Dynamic grabber statistic value; metric binding not runtime verified',15:'Dynamic grabber statistic value; metric binding not runtime verified',16:'Dynamic grabber statistic value; metric binding not runtime verified',17:'Grabber status value'},
279: {3:'Authentication challenge username/history',5:'Authentication challenge password'},
280: {2:'Batch URL wildcard pattern/history',7:'Numeric sequence first value',8:'Adjust numeric sequence first value',10:'Numeric sequence last value',11:'Adjust numeric sequence last value',13:'Numeric wildcard padding width',14:'Adjust wildcard padding width',18:'Batch HTTP username',20:'Batch HTTP password',22:'First generated URL preview',24:'Second generated URL preview',27:'Last generated URL preview',30:'Letter sequence first value',31:'Letter sequence last value'},
281: {0:'Dynamic grabber cleanup indicator',1:'Dynamic grabber cleanup indicator',2:'Dynamic grabber cleanup indicator',3:'Dynamic grabber cleanup indicator',4:'Dynamic grabber cleanup indicator'},
284: {12:'Grabber scheduled start time',17:'Grabber one-time start date',18:'Grabber repeat interval hours',20:'Grabber repeat interval minutes',31:'Grabber scheduled stop time',39:'Grabber completion command/file',40:'Choose grabber completion command/file',43:'Grabber completion power action selection'},
286: {8:'Completed download source URL',10:'Completed output filename',11:'Completed-file drag icon collection; help and screenshot/layout association, runtime wiring unverified'},
291: {4:'Configure progress dialog',16:'Duplicate URL action selection',19:'Manual download User-Agent',21:'Configure antivirus completion check'},
292: {1:'Reboot countdown progress'},294:{4:'Automatically captured file extension patterns',7:'Automatic capture excluded sites',11:'Edit automatic capture address exclusions'},
296: {2:'Discovered browser integration selection collection'},297:{0:'Browser version integration collection'},
298: {0:'Duplicate source URL'},300:{2:'Dynamic continue/cancel message'},
303: {2:'Automatic capture address exception collection'},304:{2:'Capture exception address pattern'},305:{2:'Takeover failure message'},
306: {4:'Queue scheduled start time',9:'Queue one-time start date',10:'Queue repeat interval hours',12:'Queue repeat interval minutes',22:'Queue scheduled stop time',24:'Retry count per failed queue file',26:'Queue completion command/file',27:'Choose queue completion command/file',36:'Queue completion power action selection'},
307: {1:'Concurrent files in this queue',2:'Adjust queue concurrency',4:'Ordered files assigned to queue',5:'Move item down',6:'Move item up',7:'Remove item from download queue'},
308: {9:'Selected queue settings content surface',10:'Named queue navigation tree'},309:{2:'Download quota in MBytes',5:'Quota interval in hours'},310:{0:'New queue name'},
336: {2:'Reported capture exception URL',3:'Reported capture origin/referrer URL'},337:{2:'Maximum speed for one file in KBytes/sec'},
340: {9:'Media panel captured file type collection'},341:{0:'Media panel file extension'},344:{1:'Waiting/reconnect status message'},
345: {1:'Destination queue selection',5:'Create destination queue'},348:{5:'Selected-link panel enabled site collection'},349:{2:'Selected-link panel site name'},
363: {0:'Download source URL',2:'Connection/download status value',4:'File size value',6:'Downloaded byte count',8:'Transfer rate value',10:'Time remaining value',12:'Resume capability value'},
364: {4:'Hang up modem when done',5:'Exit application when done',6:'Turn off computer when done',7:'Completion power action selection',8:'Force processes to terminate'},
365: {1:'Maximum task speed in KBytes/sec',5:'Current task transfer rate'},367:{5:'Initial progress window view'},
369: {6:'HTTP proxy address',7:'HTTP proxy port',8:'HTTP proxy username',9:'HTTP proxy password',16:'HTTPS proxy address',17:'HTTPS proxy port',18:'HTTPS proxy username',19:'HTTPS proxy password',26:'FTP proxy address',27:'FTP proxy port',28:'FTP proxy username',29:'FTP proxy password',36:'SOCKS address',37:'SOCKS port',38:'SOCKS username',39:'SOCKS password'},
370: {1:'Media panel excluded site wildcard patterns'},381:{2:'Chrome integration instruction text',3:'Chrome integration instruction text'},
487: {1:'Filename search expression',3:'Description search expression'},488:{4:'Minimum detected media size in KB',3:'Minimum media size explanation'},
489: {2:'Edge integration instruction text'},494:{3:'Antivirus scanner executable',4:'Choose antivirus scanner executable',6:'Antivirus command arguments'},
505: {1:'Quick filename search expression',4:'Close quick search bar'},506:{1:'Multi-field search expression'},
30721: {1:'Generic framework new-item list; actual caller unresolved'},
}

# Label positions independently checked against source DLU coordinates.
LABELS = {
130:{4:[3],6:[5],9:[8],14:[17]},135:{10:[9],13:[12],15:[14],21:[20],23:[22],25:[24]},
140:{12:[11],18:[14],19:[15],20:[16],21:[17],28:[27]},141:{7:[6,8],10:[9,11],14:[13]},
143:{8:[7],10:[9],15:[14]},146:{2:[1],5:[4],7:[6]},148:{1:[0],5:[4],7:[6]},
185:{3:[2],5:[4],7:[6],12:[11,13],15:[14,16]},186:{1:[0],3:[2]},204:{1:[0],3:[2],7:[6],11:[10]},
209:{8:[7,9]},226:{2:[0],5:[4]},228:{12:[20],13:[19]},232:{1:[0],3:[2],5:[4]},
240:{0:[23],2:[24],12:[11],15:[14],18:[17],20:[19]},245:{1:[0],3:[2]},
257:{1:[0],3:[2],5:[4],10:[9],12:[11],16:[15],18:[17]},262:{0:[1],2:[3]},264:{3:[4],5:[6],15:[14],17:[16]},
268:{0:[3]},269:{1:[0]},279:{3:[2],5:[4]},280:{2:[1],7:[6],10:[9],13:[12],18:[17],20:[19]},
284:{12:[11],18:[19],20:[21],31:[30],39:[38]},286:{8:[7],10:[9]},291:{16:[15],19:[18]},
294:{4:[3],7:[6]},304:{2:[3]},306:{4:[3],10:[11],12:[13],22:[21],24:[23],26:[25]},
307:{1:[0,3]},309:{2:[1,3],5:[4,6]},310:{0:[1]},336:{2:[5],3:[6]},337:{2:[4,5]},
340:{9:[8]},345:{1:[2]},364:{4:[10],5:[11],6:[12],8:[13]},365:{1:[2,3]},367:{5:[6]},
369:{6:[2],7:[3],8:[4],9:[5],16:[12],17:[13],18:[14],19:[15],26:[22],27:[23],28:[24],29:[25],36:[32],37:[33],38:[34],39:[35]},
370:{1:[0]},487:{1:[0],3:[2]},488:{4:[2,5]},494:{3:[2],6:[5]},505:{1:[0]},506:{1:[0]},
}
CLASSES = {128:'button',129:'edit',130:'static',131:'listbox',132:'scrollbar',133:'combobox'}
SUPPORT = {
    (236, 2): {
        'evidence': 'S: dialog title and adjacent capture-guidance controls; L20553 describes the associated workflow',
        'limit': 'Exact string producer and runtime trigger not traced; text meaning is mapped, caller is not.'
    },
    (286, 11): {
        'evidence': 'H: shipped CompleteDlg.htm describes a completed-file drag icon; its screenshot places the icon beside the dismissal preference, matching this small-icon collection layout',
        'help_sha256': '896dc32be0b0dc4d04029100d6f8bc0f13400e2963c5a4c232189c9f2f1ee3ea',
        'screenshot_sha256': 'cba98a3189398d766c8369f2a9c1cf15d387b38fce6bdfc5343a21c714986508',
        'limit': 'Historical help/layout association; this build\'s drag event wiring and runtime visibility not tested. Resource style does not establish owner-draw.'
    },
}

def class_name(control):
    cl = control['class']
    return CLASSES.get(cl.get('ordinal'),f"ordinal-{cl.get('ordinal')}") if isinstance(cl,dict) else cl

def role(control):
    cl = class_name(control)
    subtype = control['style'] & 15
    if cl == 'button':
        if subtype == 7: return 'group-container'
        if subtype in [2,3,5,6]: return 'boolean-choice'
        if subtype in [4,9]: return 'exclusive-choice'
        return 'action'
    if cl == 'edit': return 'read-only-output' if control['style'] & 0x800 else 'input'
    if cl == 'combobox': return 'choice-or-input'
    if cl in ['SysListView32','SysTreeView32','listbox']:return 'collection'
    if cl == 'SysDateTimePick32':return 'date-time-input'
    if cl == 'msctls_updown32':return 'numeric-adjustment'
    if cl == 'msctls_progress32':return 'progress-output'
    if cl == 'static':
        subtype = control['style'] & 31
        if subtype in [3,14]:return 'icon-or-image'
        if subtype in [7,8,9,16,17,18]:return 'decoration'
        return 'text-output-or-label'
    return 'unresolved-control-class'

def clean(value):
    return value.replace('&','').replace('\\r\\n',' ').replace('\\n',' ').replace('\r',' ').replace('\n',' ').strip() if isinstance(value,str) else ''

def generate():
    source_bytes=SOURCE.read_bytes();source=json.loads(source_bytes)
    language=json.loads(LANG.read_text())['entries']; review=json.loads(REVIEW.read_text())
    by_text={};by_key={}
    for item in language:
        by_text.setdefault(clean(item['text']),[]).append(item)
        by_key[item['key']]=item
    refs={r['source_key']:r for r in review['references']}
    source_contracts={}
    for r in review['references']:
        source_contracts[r['catalog_key']]=r['specific_contracts']
    families={did:spec[0] for spec in __import__('build_feature_coverage').SPECS for did in spec[1]}
    dialogs=[]; counts=Counter();keys=set()
    for d in source['dialogs']:
        did=d['resource_id'];records=[]
        for index,c in enumerate(d['controls']):
            key=f'D{did}/C{index}';assert key not in keys;keys.add(key)
            kind=role(c);caption=clean(c['title']);semantic=BINDINGS.get(did,{}).get(index)
            labels=LABELS.get(did,{}).get(index,[])
            matches=list(by_text.get(caption,[])) if caption else []
            for label_index in labels:
                matches.extend(by_text.get(clean(d['controls'][label_index]['title']),[]))
            tooltip=None
            if kind in ['action','boolean-choice','exclusive-choice']:
                tooltip=by_key.get(f"t{c['id']}")
                if tooltip:matches.append(tooltip)
            if tooltip and not caption:
                semantic=tooltip['text'];evidence='S: shipped tooltip; association by same control ID is a candidate until caller verification'
            elif semantic:
                evidence='S: manually reviewed label/layout binding; association inferred, not runtime observed'
            elif caption and caption not in ['Static','text','Some String'] and not caption.startswith(('List','Tree','Spin','Progress','DateTimePicker')):
                semantic=caption;evidence='S: explicit resource caption'
            elif kind in ['decoration','group-container','icon-or-image']:
                semantic='Resource layout/container/image; no separate executable action established';evidence='S: resource class/style'
            else:
                semantic='Unresolved dynamic content or action; caller/content requires inspection';evidence='S: control identity only'
            unresolved=semantic.startswith('Unresolved') or 'unresolved' in semantic.lower()
            counts['controls']+=1;counts[kind]+=1
            counts['unresolved_semantics']+=unresolved
            if unresolved and kind in ['input','action','choice-or-input','collection','boolean-choice','exclusive-choice','date-time-input','numeric-adjustment']:counts['unresolved_interactive_semantics']+=1
            references=sorted({f"L{m['key']}@{m['line']}" for m in matches})
            contracts=sorted({cid for m in matches for cid in source_contracts.get(m['key'],[])})
            record={
              'key':key,'source_id':c['id'],'source_index':index,'source_class':c['class'],'source_style':c['style'],'source_ex_style':c['ex_style'],'source_bounds_dlu':c['bounds_dlu'],'source_caption':c['title'],
              'role':kind,'semantic_binding':semantic,'evidence_level':evidence,'binding_label_keys':[f'D{did}/C{x}' for x in labels],
              'additional_binding_support':SUPPORT.get((did,index)),
              'binding_label_texts':[d['controls'][x]['title'] for x in labels],
              'catalog_caption_or_tooltip_candidates':references,'behavior_contract_candidates':contracts,'target_family':families[did],
              'mapping_state':'unresolved-semantic' if unresolved else 'static-semantic-mapped',
              'runtime_caller':'not-established-by-this-review','runtime_default':'not-established','enabled_visibility_conditions':'resource_style_preserved; runtime conditions unknown',
              'xdm_control_parity':'requires-individual-implementation-and-verification',
              'acceptance_requirement':acceptance(kind,semantic,unresolved)
            }
            records.append(record)
        dialogs.append({'key':f'D{did}','resource_id':did,'source_language':d['language'],'source_title':d['title'],'source_style':d['style'],'source_ex_style':d['ex_style'],'source_bounds_dlu':d['bounds_dlu'],'target_family':families[did],'reachability':'resource-presence; not every template proven reachable','controls':records})
    assert len(dialogs)==105 and counts['controls']==1055
    for did,bindings in BINDINGS.items():
        dialog=next(d for d in source['dialogs'] if d['resource_id']==did)
        assert all(0<=i<len(dialog['controls']) for i in bindings)
    counts['dialogs']=len(dialogs);counts['explicit_manual_bindings']=sum(map(len,BINDINGS.values()))
    result={'schema_version':1,'scope':'Every extracted dialog/control individually classified. Static semantics and manually inferred layout bindings do not establish runtime behavior, defaults, callers or implemented parity. Source text is audit evidence, not XDM UI copy.','source':str(SOURCE.relative_to(ROOT)),'source_sha256':hashlib.sha256(source_bytes).hexdigest(),'behavior_review_sha256':hashlib.sha256(REVIEW.read_bytes()).hexdigest(),'counts':dict(counts),'dialogs':dialogs}
    md=['# IDM individual control review','',result['scope'],'',f"Inventory: {len(dialogs)} dialogs, {counts['controls']} controls; {counts['explicit_manual_bindings']} manually bound input/collection/output controls; {counts['unresolved_interactive_semantics']} interactive semantics unresolved.",'','Caption matches and reused tooltip IDs are contract candidates. Caller/default/visibility and action effects require separate runtime verification. Numeric Win32 styles are preserved; initial checked state is not inferred from checkbox type.','','D236/C2 is capture-guidance text; the exact runtime producer remains unknown. D286/C11 is associated with the completed-file drag icon using shipped `CompleteDlg.htm` and its screenshot/layout; this is H/layout evidence, not tested event wiring. The JSON retains the help/screenshot hashes. D30721/C1 remains a generic framework-looking New-item collection with no established product caller.','']
    for d in dialogs:
        md.extend([f"## {d['key']} — {d['source_title'] or '(untitled template)'}",'',f"Target family: `{d['target_family']}`. Reachability: {d['reachability']}.",'','| Control | Source ID | Role | Semantic binding | Evidence / labels |','|---|---:|---|---|---|'])
        for c in d['controls']:
            safe=lambda s:str(s).replace('|','\\|').replace('\n',' ')
            ev=c['evidence_level']+(' '+','.join(c['binding_label_keys']) if c['binding_label_keys'] else '')
            md.append(f"| {c['key']} | {c['source_id']} | {c['role']} | {safe(c['semantic_binding'])} | {safe(ev)} |")
        md.append('')
    return result,'\n'.join(md)+'\n'

def acceptance(kind,semantic,unresolved):
    if unresolved:return 'Establish caller/content and reachable workflow before defining equivalent behavior; retain source identity in coverage.'
    if kind in ['input','choice-or-input','date-time-input','numeric-adjustment']:
        return f'For {semantic}: establish accepted values, validation, cancellation, persistence and runtime enable conditions; test resulting task/settings behavior, not only rendering.'
    if kind in ['boolean-choice','exclusive-choice']:
        return f'For {semantic}: establish default, dependency rules, Apply/Cancel persistence and both choice outcomes in original workflow.'
    if kind=='action':return f'For {semantic}: establish triggering context, selection/preconditions, successful effect, cancellation and error outcome.'
    if kind=='collection':return f'For {semantic}: establish columns/items, selection model, ordering, empty/error states and contextual actions.'
    if kind in ['progress-output','read-only-output','text-output-or-label']:
        return f'For {semantic}: distinguish fixed label from runtime output, establish update source and formatting; verify unknown/empty/error presentation.'
    return 'Preserve accessibility/layout purpose with independent XDM design; prove any runtime interaction before treating decorative resource as a feature.'

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
    result,md=generate();artifacts={ROOT/'docs/idm_control_review.json':json.dumps(result,ensure_ascii=False,indent=2)+'\n',ROOT/'docs/idm_control_review.md':md}
    for path,text in artifacts.items():
        if args.check:
            if not path.exists() or path.read_text()!=text:raise SystemExit(f'Stale artifact: {path}')
        else:path.write_text(text)
    print(json.dumps(result['counts'],sort_keys=True))
