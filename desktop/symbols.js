import {element} from './dom.js';
import {t, currentLanguage} from './i18n.js';

const groups = [
  ['※', '참고표', 'Reference mark'],
  ['○●◎', '동그라미', 'Circle'],
  ['□■', '네모', 'Square'],
  ['△▲▶▷', '삼각형', 'Triangle'],
  ['◆◇', '마름모', 'Diamond'],
  ['·ㆍ', '가운뎃점', 'Middle dot'],
  ['∼', '물결', 'Tilde'],
  ['「」『』〈〉《》【】', '괄호 낫표 겹낫표', 'Bracket quotation'],
  ['①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮⑯⑰⑱⑲⑳', '원 숫자', 'Circled number'],
  ['㉠㉡㉢㉣㉤㉥㉦㉧㉨㉩㉪㉫㉬㉭', '원 한글', 'Circled Hangul'],
  ['㈀㈁㈂㈃㈄㈅㈆㈇㈈㈉㈊㈋㈌㈍', '괄호 한글', 'Parenthesized Hangul'],
  ['ⅠⅡⅢⅣⅤⅥⅦⅧⅨⅩ', '로마 숫자', 'Roman numeral'],
  ['℃', '섭씨', 'Celsius'], ['㎡', '제곱미터', 'Square metre'],
  ['㎞', '킬로미터', 'Kilometre'], ['㎏', '킬로그램', 'Kilogram'],
  ['₩', '원화', 'Won'], ['±', '플러스 마이너스', 'Plus minus'],
  ['×', '곱하기', 'Multiply'], ['÷', '나누기', 'Divide'],
  ['→←↑↓⇒⇔', '화살표', 'Arrow']
];
export const SYMBOLS = groups.flatMap(([symbols, ko, en]) => [...symbols].map(symbol => ({symbol, ko, en})));
export function searchSymbols(query) {
  const needle = query.trim().normalize('NFKC').toLowerCase();
  return SYMBOLS.filter(item => `${item.symbol} ${item.ko} ${item.en}`.normalize('NFKC').toLowerCase().includes(needle));
}
export function insertSymbol(text, start, end, symbol) {
  return {text: text.slice(0, start) + symbol + text.slice(end), caret: start + symbol.length};
}
export function rememberSymbol(recent, symbol) {
  return [...new Set([symbol, ...recent])].filter(value => SYMBOLS.some(item => item.symbol === value)).slice(0, 16);
}
export class SymbolPicker {
  constructor(dialog, insert, storage = localStorage) {
    this.dialog = dialog;
    this.insert = insert;
    this.storage = storage;
    try {
      const saved = JSON.parse(storage.getItem('hwp-recent-symbols') || '[]');
      this.recent = Array.isArray(saved) ? saved.filter(value => SYMBOLS.some(item => item.symbol === value)).slice(0, 16) : [];
    } catch {
      this.recent = [];
    }
  }
  open() {
    this.focus = document.activeElement;
    const header = element('header');
    header.append(element('strong', '', t('symbols')));
    const close = element('button', 'btn ghost', '×');
    close.setAttribute('aria-label', t('close'));
    close.onclick = () => this.dialog.close();
    header.append(close);
    this.query = element('input', 'symbol-query');
    this.query.id = 'symbolQuery';
    this.query.setAttribute('aria-label', t('symbolSearch'));
    this.query.placeholder = t('symbolSearch');
    this.query.oninput = () => this.render();
    this.results = element('div', 'symbol-grid');
    this.dialog.replaceChildren(header, this.query, this.results);
    this.dialog.onclose = () => this.focus?.focus({preventScroll: true});
    this.dialog.showModal();
    this.render();
    this.query.focus();
  }
  render() {
    const matches = searchSymbols(this.query.value);
    if (!this.query.value) {
      matches.sort((a, b) => {
        const rank = symbol => this.recent.includes(symbol) ? this.recent.indexOf(symbol) : 100;
        return rank(a.symbol) - rank(b.symbol);
      });
    }
    this.results.replaceChildren(...matches.map(item => {
      const button = element('button', 'symbol-choice', item.symbol);
      button.title = item[currentLanguage()];
      button.setAttribute('aria-label', `${item.symbol} · ${item[currentLanguage()]}`);
      button.onclick = () => {
        this.dialog.close();
        this.insert(item.symbol);
        this.recent = rememberSymbol(this.recent, item.symbol);
        try {
          this.storage.setItem('hwp-recent-symbols', JSON.stringify(this.recent));
        } catch {
          // A full preferences store must not prevent text insertion.
        }
      };
      return button;
    }));
  }
}
