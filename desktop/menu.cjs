// Load the same ES-module key table used by the renderer.
async function installMenu({Menu, app, win}, scheme) {
  const {acceleratorFor} = await import('./keymap.js');
  const labels = {
    open: 'Open / 불러오기', save: 'Save / 저장', saveCopy: 'Save As / 다른 이름으로 저장',
    undo: 'Undo / 되돌리기', redo: 'Redo / 다시 실행', find: 'Find / 찾기', replace: 'Find and replace / 찾아 바꾸기',
    palette: 'Commands / 명령', shortcuts: 'Keyboard shortcuts / 단축키'
  };
  const item = id => ({id, label: labels[id], accelerator: acceleratorFor(id, scheme),
    click: () => win.webContents.send('app:command', id)});
  Menu.setApplicationMenu(Menu.buildFromTemplate([
    {label: app.name, submenu: [{role: 'about'}, {type: 'separator'}, {role: 'quit'}]},
    {label: 'File / 파일', submenu: ['open', 'save', 'saveCopy'].map(item)},
    {label: 'Edit / 편집', submenu: [item('undo'), item('redo'), {type: 'separator'},
      {role: 'cut'}, {role: 'copy'}, {role: 'paste'}, {role: 'selectAll'}, item('find'), item('replace')]},
    {label: 'View / 보기', submenu: [item('palette'), item('shortcuts'),
      {label: 'Language / 언어', click: () => win.webContents.send('app:command', 'language')}, {role: 'togglefullscreen'}]}
  ]));
}
module.exports = {installMenu};
