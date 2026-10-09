#!/usr/bin/env python3
"""Fetch fixed Git blobs, verify blob identity AND size, then record SHA-256.
No document bytes are uploaded. Public test fixtures remain gitignored.
"""
import base64,hashlib,json,sys,urllib.request
from pathlib import Path
root=Path(__file__).resolve().parents[1];sys.path.insert(0,str(root/'engine'))
from hwp_workbench.workspace import Workspace
manifest=json.loads((root/'fixtures/public-manifest.json').read_text())
folder=root/'fixtures/public';folder.mkdir(parents=True,exist_ok=True);ws=Workspace(folder)
results=[]
for item in manifest['fixtures']:
    try:
        dest=folder/item['name']
        if dest.exists():data=dest.read_bytes()
        else:
            url=f"https://api.github.com/repos/{item['repository']}/git/blobs/{item['blobSha1']}"
            req=urllib.request.Request(url,headers={'Accept':'application/vnd.github+json','User-Agent':'hwp-workbench-fixture-fetch/0.1'})
            with urllib.request.urlopen(req,timeout=20) as response:record=json.loads(response.read(4*1024*1024))
            if record.get('encoding')!='base64':raise ValueError('Unexpected blob encoding')
            data=base64.b64decode(record['content'])
        actual=hashlib.sha1(f'blob {len(data)}\0'.encode()+data).hexdigest()
        if actual!=item['blobSha1'] or len(data)!=item['bytes']:raise ValueError('Blob identity or byte count mismatch')
        if not dest.exists():ws.write_new(item['name'],data)
        results.append({'name':item['name'],'status':'verified','sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data)})
        print('Verified:',item['name'])
    except Exception as e:
        results.append({'name':item['name'],'status':'download-failed','error':str(e)});print('Not downloaded:',item['name'],str(e),file=sys.stderr)
(root/'reports/public-fixture-downloads.json').write_text(json.dumps(results,indent=2))
sys.exit(0 if all(x['status']=='verified' for x in results) else 1)
