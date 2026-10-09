const {app,BrowserWindow,dialog,ipcMain,session,clipboard,Menu}=require('electron');
const {spawn}=require('node:child_process');
const fs=require('node:fs/promises');
const fsSync=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const {pathToFileURL}=require('node:url');
const ROOT=path.resolve(__dirname,'..');
const INDEX=pathToFileURL(path.join(__dirname,'index.html')).href;
let win,current=null,queue=Promise.resolve();
function callEngine(root,method,args){
 const payload=JSON.stringify({method,args});
 if(Buffer.byteLength(payload)>4*1024*1024)return Promise.reject(Error('Request too large.'));
 return new Promise((resolve,reject)=>{
  const python=process.env.HWP_WORKBENCH_PYTHON || ['/opt/homebrew/bin/python3','/usr/local/bin/python3','/usr/bin/python3'].find(p=>fsSync.existsSync(p)) || 'python3';
  const child=spawn(python,['-m','hwp_workbench','rpc','--root',root],{cwd:ROOT,shell:false,env:{...process.env,PYTHONPATH:path.join(ROOT,'engine'),PYTHONUTF8:'1'},stdio:['pipe','pipe','pipe']});
  let out='',err='',size=0,settled=false;
  const timer=setTimeout(()=>{child.kill('SIGKILL');finish(Error('Document processing exceeded the 30-second limit.'));},30000);
  function finish(error,value){if(settled)return;settled=true;clearTimeout(timer);error?reject(error):resolve(value);}
  child.on('error',e=>finish(Error(`Python 3.11+ is required. Set HWP_WORKBENCH_PYTHON to its absolute path. ${e.message}`)));
  child.stdout.setEncoding('utf8');child.stderr.setEncoding('utf8');
  child.stdout.on('data',b=>{size+=Buffer.byteLength(b);if(size>32*1024*1024){child.kill('SIGKILL');finish(Error('Response too large.'));}else out+=b;});
  child.stderr.on('data',b=>{if(err.length<65536)err+=b;});
  child.stdin.on('error',()=>{});
  child.on('close',code=>{if(code!==0){let message=err.trim();try{message=JSON.parse(err).error.message;}catch{}finish(Error(message||'Engine failed.'));return;}try{finish(null,JSON.parse(out));}catch{finish(Error('Invalid engine response.'));}});
  child.stdin.end(payload);
 });
}
function guard(event){if(!win||event.sender!==win.webContents||event.senderFrame?.url!==INDEX)throw Error('Unauthorized IPC sender.');}
function validatePatch(p){if(!p||p.schemaVersion!==1||p.baseSha256!==current?.sha256||!Array.isArray(p.operations)||p.operations.length>1000)throw Error('Invalid or stale patch.');if(Buffer.byteLength(JSON.stringify(p))>2*1024*1024)throw Error('Patch too large.');return p;}
const draftPath=()=>path.join(app.getPath('userData'),'drafts',current.sha256+'.json');
async function load(file){
 const root=path.dirname(file);const document=await callEngine(root,'inspect',{path:path.basename(file)});
 current={file,root,sha256:document.sha256};let draft=null;
 try{draft=JSON.parse(await fs.readFile(draftPath(),'utf8'));}catch{}
 return {document,draft};
}
function register(channel,fn){ipcMain.handle(channel,(event,...args)=>{guard(event);const task=queue.then(()=>fn(...args));queue=task.catch(()=>{});return task;});}
register('document:open',async()=>{const result=await dialog.showOpenDialog(win,{properties:['openFile'],filters:[{name:'Korean documents / 한글 문서',extensions:['hwp','hwpx']}]});return result.canceled?null:load(result.filePaths[0]);});
register('document:demo',()=>load(path.join(ROOT,'fixtures','generated','review-form.hwpx')));
register('document:plan',async p=>{validatePatch(p);return callEngine(current.root,'plan',{path:path.basename(current.file),patch:p});});
register('document:save',async(p,ack)=>{
 validatePatch(p);if(ack!==true)throw Error('Acknowledge compatibility risks first.');
 const ext=path.extname(current.file);const defaultPath=current.file.slice(0,-ext.length)+'-edited'+ext;
 const result=await dialog.showSaveDialog(win,{defaultPath,filters:[{name:'Native document',extensions:[ext.slice(1)]}]});
 if(result.canceled)return null;
 // Save is a native-dialog grant, not a path supplied by renderer content.
 const output=path.resolve(result.filePath);
 const from=path.resolve(current.file).split(path.sep),to=output.split(path.sep);let i=0;while(i<from.length-1&&i<to.length-1&&from[i]===to[i])i++;
 const root=from.slice(0,i).join(path.sep)||path.parse(current.file).root;
 const report=await callEngine(root,'apply',{path:path.relative(root,current.file),output:path.relative(root,output),patch:p,acknowledgeRisks:true});
 return {...report,output};
});
register('document:export-patch',async p=>{validatePatch(p);const result=await dialog.showSaveDialog(win,{defaultPath:current.file+'.patch.json',filters:[{name:'JSON patch',extensions:['json']}]});if(result.canceled)return null;await fs.writeFile(result.filePath,JSON.stringify(p,null,2),{flag:'wx',mode:0o600});return true;});
register('document:draft',async p=>{validatePatch(p);const dest=draftPath();await fs.mkdir(path.dirname(dest),{recursive:true,mode:0o700});const temp=dest+'.'+crypto.randomBytes(8).toString('hex')+'.tmp';try{await fs.writeFile(temp,JSON.stringify(p),{flag:'wx',mode:0o600});await fs.rename(temp,dest);}finally{await fs.rm(temp,{force:true});}return true;});
register('clipboard:copy',async text=>{if(typeof text!=='string'||text.length>4096)throw Error('Invalid clipboard text.');clipboard.writeText(text);return true;});
app.whenReady().then(()=>{
 session.defaultSession.setPermissionRequestHandler((_wc,_permission,callback)=>callback(false));
 session.defaultSession.setPermissionCheckHandler(()=>false);
 // The app has no remote content, telemetry, or HTTP service.
 session.defaultSession.webRequest.onBeforeRequest({urls:['http://*/*','https://*/*','ws://*/*','wss://*/*']},(_details,callback)=>callback({cancel:true}));
 win=new BrowserWindow({width:1400,height:930,minWidth:980,minHeight:690,title:'HWP Workbench',backgroundColor:'#f3f5f1',webPreferences:{preload:path.join(__dirname,'preload.cjs'),nodeIntegration:false,contextIsolation:true,sandbox:true,webSecurity:true,allowRunningInsecureContent:false,spellcheck:false}});
 win.webContents.setWindowOpenHandler(()=>({action:'deny'}));win.webContents.on('will-navigate',e=>e.preventDefault());
 win.webContents.on('will-prevent-unload',async()=>{const r=await dialog.showMessageBox(win,{type:'question',buttons:['Keep editing / 계속 편집','Close / 닫기'],defaultId:0,cancelId:0,message:'Close with unsaved edits? / 저장하지 않고 닫을까요?',detail:'Staged edits are retained in the local draft. Unstaged text is not. / 적용한 변경만 로컬 초안에 보관됩니다.'});if(r.response===1)win.destroy();});
 Menu.setApplicationMenu(Menu.buildFromTemplate([{label:app.name,submenu:[{role:'about'},{type:'separator'},{role:'quit'}]},{label:'Edit / 편집',submenu:[{role:'undo'},{role:'redo'},{type:'separator'},{role:'cut'},{role:'copy'},{role:'paste'},{role:'selectAll'}]},{label:'View / 보기',submenu:[{role:'togglefullscreen'}]}]));
 win.loadURL(INDEX);
});
app.on('window-all-closed',()=>app.quit());
