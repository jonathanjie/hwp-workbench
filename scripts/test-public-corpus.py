#!/usr/bin/env python3
"""Test public fixture bytes locally; never treat absent samples as passing."""
import json,sys,hashlib
from pathlib import Path
root=Path(__file__).resolve().parents[1];sys.path.insert(0,str(root/'engine'))
from hwp_workbench.document import Document,sha
from hwp_workbench.errors import WorkbenchError
manifest=json.loads((root/'fixtures/public-manifest.json').read_text());results=[]
for item in manifest['fixtures']:
    path=root/'fixtures/public'/item['name']
    r={'name':item['name'],'hancomCertified':False}
    if not path.exists():r.update(status='not-run',reason='Fixture not downloaded.');results.append(r);continue
    try:
        raw=path.read_bytes()
        blob=hashlib.sha1(f'blob {len(raw)}\0'.encode()+raw).hexdigest()
        if blob!=item['blobSha1'] or len(raw)!=item['bytes']:raise ValueError('Fixture identity/size differs from manifest.')
        d=Document(raw,path.name)
        r.update(status='parsed',textNodes=len(d.nodes),warnings=[w['code'] for w in d.warnings],noopByteIdentical=d.serialize()==raw)
        candidates=[n for n in d.nodes if n.editable and n.text and all(ord(c)>=32 for c in n.text)]
        if candidates:
            n=candidates[0];text=n.text[:-1]+('가' if n.text[-1]!='가' else '나')
            if d.format=='hwp' and len(text.encode('utf-16le'))!=len(n.text.encode('utf-16le')):r['edit']='not-run: no safe length-preserving test'
            else:
                d.patch({'schemaVersion':1,'baseSha256':d.hash,'operations':[{'op':'set_text','nodeId':n.id,'expectedText':n.text,'text':text}]})
                b=d.serialize(True);again=Document(b)
                assert again.by_id[n.id].text==text
                r['edit']='reopened and structurally verified';r['editedSha256']=sha(b)
        else:r['edit']='not-run: no safely editable text'
    except Exception as e:r.update(status='failed',error=str(e))
    results.append(r)
(root/'reports/public-corpus-results.json').write_text(json.dumps(results,indent=2));print(json.dumps(results,indent=2))
sys.exit(0 if all(r['status']=='parsed' for r in results) else 1)
