#!/usr/bin/env python3
"""Chromium UI smoke test with real Python engine and a test-only IPC bridge.
Does NOT test Electron, native macOS dialogs, macOS packaging, or real IME.
Requires optional Playwright Python package and Chromium on the test host.
"""
from pathlib import Path
import json,sys,tempfile,shutil,re
root=Path(__file__).resolve().parents[1];sys.path.insert(0,str(root/'engine'))
from hwp_workbench.service import invoke
from playwright.sync_api import sync_playwright
reports=root/'reports';work=reports/'ui-test-workspace';work.mkdir(exist_ok=True)
for f in (root/'fixtures/generated').iterdir():shutil.copyfile(f,work/f.name)
for f in work.glob('*-edited.hwpx'):f.unlink()
state={'path':'review-form.hwpx','draft':None,'copy':None}
checks=[];errors=[]
def bridge(request):
    method=request["method"];args=request["args"]
    if method in {'open','openDemo'}:
        return {'document':invoke(work,'inspect',{'path':state['path']}),'draft':None}
    if method=='plan':return invoke(work,'plan',{'path':state['path'],'patch':args[0]})
    if method=='saveCopy':return invoke(work,'apply',{'path':state['path'],'patch':args[0],'output':'review-form-edited.hwpx','acknowledgeRisks':args[1]})
    if method=='saveDraft':state['draft']=args[0];return True
    if method=='copyText':state['copy']=args[0];return True
    if method=='exportPatch':state['patch']=args[0];return True
    raise ValueError(method)
with sync_playwright() as p:
    browser=p.chromium.launch(executable_path=shutil.which('chromium'),headless=True,args=['--no-sandbox'])
    page=browser.new_page(viewport={'width':1440,'height':1000},device_scale_factor=1)
    page.set_default_timeout(5000)
    page.on('pageerror',lambda e:errors.append(str(e)))
    page.on('dialog',lambda d:d.accept())
    page.expose_function('testBridge',bridge)
    # Render our local sources in memory: this host blocks URL navigation.
    # This exercises the renderer, not Electron's CSP enforcement or file loading.
    html=(root/'desktop/index.html').read_text()
    html=re.sub(r'<link rel="stylesheet"[^>]*>', '<style>'+(root/'desktop/styles.css').read_text()+'</style>',html)
    html=re.sub(r'<script type="module" src="renderer.js"></script>','',html)
    page.set_content(html)
    page.evaluate("Object.defineProperty(window,'localStorage',{value:{getItem:()=>null,setItem:()=>{}}})")
    page.evaluate("window.hwp=Object.fromEntries(['open','openDemo','plan','saveCopy','saveDraft','copyText','exportPatch'].map(m=>[m,(...a)=>window.testBridge({method:m,args:a})]))")
    page.evaluate('() => {'+(root/'desktop/renderer.js').read_text()+'}')
    page.click('#demo');page.wait_for_selector('.text-node')
    assert page.locator('#filename').inner_text()=='review-form.hwpx';checks.append('Open HWPX through real engine')
    assert page.locator('.doc-table').count()==1;checks.append('Table structure displayed')
    assert page.locator('.warning').count()>=4;checks.append('Compatibility warnings displayed')
    target=page.locator('.text-node').filter(has_text='마리나체인').first;target.click()
    assert page.locator('#textEdit').input_value()=='마리나체인';checks.append('Select a table-cell text node')
    page.fill('#textEdit','MarinaChain');page.click('#applyText');page.wait_for_timeout(100)
    assert page.locator('.text-node.edited').inner_text()=='MarinaChain';checks.append('Stage variable-length HWPX text edit')
    assert state['draft'] and len(state['draft']['operations'])==1;checks.append('Autosave staged patch via bridge')
    page.click('#undo');assert page.locator('.text-node.edited').count()==0
    page.click('#redo');assert page.locator('.text-node.edited').count()==1;checks.append('Staged undo and redo')
    page.click('#copyId');page.wait_for_timeout(80);assert state['copy'] and '#text:' in state['copy'];checks.append('Copy stable node ID')
    page.fill('#search','MarinaChain');assert page.locator('.outline-item').count()==1;page.fill('#search','');checks.append('Search outline')
    page.click('#review');page.wait_for_selector('#reviewDialog[open]');assert page.locator('.change').count()==1
    assert page.locator('#confirmSave').is_disabled();checks.append('Review requires explicit risk acknowledgement')
    page.screenshot(path=str(reports/'change-review.png'))
    page.check('#acknowledge');page.click('#confirmSave');page.locator("#reviewDialog[open]").wait_for(state="hidden")
    assert (work/'review-form-edited.hwpx').exists();checks.append('Save new native file and verify reopen using real engine')
    page.click('#language');assert page.locator('#open').inner_text()=='문서 열기';assert page.locator('#filename').inner_text()=='review-form.hwpx';checks.append('Korean UI switch preserves document state')
    page.screenshot(path=str(reports/'editor-korean.png'))
    page.click('#language');page.screenshot(path=str(reports/'editor-english.png'))
    page.fill('#textEdit','Unstaged buffer');page.click('#language');assert page.locator('#textEdit').input_value()=='Unstaged buffer';page.click('#language');checks.append('Language switch retains unstaged typing')
    state['path']='legacy-review.hwp';page.click('#open');page.locator('#filename:text-is("legacy-review.hwp")').wait_for()
    page.locator('.text-node').first.click();page.fill('#textEdit','짧음');assert page.locator('#applyText').is_disabled();checks.append('HWP length-changing edit blocked in UI')
    page.fill('#textEdit','마리나체인 검토 완');assert page.locator('#applyText').is_enabled();checks.append('HWP equal-length edit enabled')
    browser.close()
report={'testHost':'Linux Chromium with real Python engine; test-only IPC adapter','passedChecks':len(checks),'checks':checks,'browserErrors':errors,'notTested':['Electron runtime, CSP enforcement, source file loading, and real preload IPC','macOS native dialogs','real Korean IME composition','Hancom layout parity']}
(reports/'ui-test-results.json').write_text(json.dumps(report,indent=2));print(json.dumps(report,indent=2));sys.exit(1 if errors else 0)
