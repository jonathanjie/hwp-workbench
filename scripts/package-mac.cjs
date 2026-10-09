/** Build an ad-hoc-signed developer .app from the installed arm64 Electron runtime.
 * Python 3.11+ remains an external prerequisite in this alpha.
 */
const fs=require('node:fs/promises');const path=require('node:path');const {execFileSync}=require('node:child_process');
(async()=>{
 if(process.platform!=='darwin'||process.arch!=='arm64')throw Error('Run this build on Apple Silicon macOS with arm64 Node.js.');
 const root=path.resolve(__dirname,'..');const electron=path.dirname(require.resolve('electron/package.json'));
 const source=path.join(electron,'dist','Electron.app');await fs.access(source);
 const target=path.join(root,'dist','mac-arm64','HWP Workbench.app');
 await fs.mkdir(path.dirname(target),{recursive:true});
 try{await fs.access(target);throw Error('Build destination exists. Move it aside before rebuilding: '+target);}catch(e){if(e.code!=='ENOENT')throw e;}
 await fs.cp(source,target,{recursive:true});
 const resources=path.join(target,'Contents','Resources');const appDir=path.join(resources,'app');await fs.mkdir(appDir,{recursive:true});
 for(const file of ['package.json','desktop','engine','fixtures/generated'])await fs.cp(path.join(root,file),path.join(appDir,file),{recursive:true,filter:src=>!src.includes('__pycache__')});
 await fs.rm(path.join(resources,'default_app.asar'),{force:true});
 const plist=path.join(target,'Contents','Info.plist');
 const metadata={CFBundleName:'HWP Workbench',CFBundleDisplayName:'HWP Workbench',CFBundleIdentifier:'internal.hwpworkbench.app',CFBundleShortVersionString:'0.1.0',CFBundleVersion:'1'};
 for(const [key,value] of Object.entries(metadata)){
  let exists=true;
  try{execFileSync('/usr/libexec/PlistBuddy',['-c',`Print :${key}`,plist],{stdio:'pipe'});}catch{exists=false;}
  const command=exists?`Set :${key} ${value}`:`Add :${key} string ${value}`;
  execFileSync('/usr/libexec/PlistBuddy',['-c',command,plist],{stdio:'pipe'});
 }
 execFileSync('/usr/bin/codesign',['--force','--deep','--sign','-',target],{stdio:'inherit'});
 console.log(target+'\nDeveloper build only: ad-hoc signed, not notarized, and requires Python 3.11+.');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
