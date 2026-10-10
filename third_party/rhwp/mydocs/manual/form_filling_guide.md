---
kind: guide
status: active
canonical: mydocs/manual/form_filling_guide.md
last_verified: 2026-07-30
---

# 서식 자동화 심화 가이드 — 실물 양식을 사람이 한 것처럼 채우기

`edit fill-fields`(누름틀) · `edit set-cell`(표 좌표) · `edit replace-text`(문구 치환)
3축의 **심화 의미론**을 공식화한다. 명령·옵션의 canonical reference 는
[CLI 명령어 매뉴얼](cli_commands.md)이고, 첫 레시피는
[에이전트 실무 대체 예제집](agent_task_playbook.md) 1절이다. 본 문서는 그 다음 —
실물 서식에서 항상 만나는 함정과 판정 규칙을 다룬다.

핵심 원칙 하나로 요약된다: **"값이 들어갔다"와 "사람이 제출할 수 있는 산출물이다"는
다른 명제이고, 각각 별도의 기계 검증이 있다.**

## 0. 축 선택 — 이 서식은 무엇으로 채우나

| 서식의 실체 | 축 | 판별 방법 |
|---|---|---|
| 누름틀(클릭해서 입력하는 칸)이 있다 | `fill-fields` | `fields --json` 의 `fieldCount ≥ 1` |
| 누름틀 없이 표의 빈 칸에 직접 쓴다 | `set-cell` | `fields` 가 0건인데 `export-tables --json` 에 빈 셀 |
| 완성 문서의 문구만 바꾼다(연도·기관명) | `replace-text` | 위 둘이 아니고 대상이 본문 문자열 |

실물 서식은 셋이 섞여 있는 경우가 많다 — 관공서 양식의 머리 표는 누름틀,
본문 표는 맨 셀인 식이다. 축별로 나눠 처리하고 마지막에 한 번에 검증한다.

## 1. `fill-fields` 심화

### 1-1. 반복 필드 — 같은 이름이 여러 번 나올 때 (`이름[N]`)

규제영향분석서·사업계획서류 서식은 `성명`·`날짜` 같은 이름이 문서에 여러 번 나온다.
순번 없이 주면 **첫 번째만 채워지고**, 응답의 `ambiguous` 에 몇 개 중 몇 개가
채워졌는지 보고된다 — 이것을 침묵 성공으로 오독하는 것이 실무 1순위 함정이다.

```bash
# ① 순번 확인 — fields 목록 순서가 곧 순번(0 기준)
rhwp fields 분석서.hwp --json | jq -r '.fields[] | .name' | sort | uniq -c | sort -rn

# ② 반복 이름은 [N] 으로 지목한다
cat > row.json <<'JSON'
{ "성명[0]": "홍길동", "성명[1]": "김가온", "날짜": "2026. 7. 30." }
JSON
rhwp edit fill-fields 분석서.hwp --data @row.json -o out.hwp --json \
  | jq -c '{filledCount, notFound, ambiguous}'
```

판정 규칙: `ambiguous` 가 비어 있지 않으면 **아직 끝난 게 아니다.** 남은 순번을
`[N]` 으로 재지목해 다시 채운다.

### 1-2. `notFound` — 오타는 조용히 무시되지 않는다

문서에 없는 필드 이름은 `notFound` 배열로 보고된다. `filledCount` 만 보고 넘어가면
오타 난 필드가 빈 채로 제출된다. 게이트는 이렇게 세운다:

```bash
rhwp edit fill-fields in.hwp --data @row.json -o out.hwp --json \
  | jq -e '(.notFound | length == 0) and (.ambiguous | length == 0)' > /dev/null \
  || { echo "미해결 필드 존재 — 제출 불가"; exit 1; }
```

### 1-3. dry-run → 실채움 → 재독 — 3단 검증 루프

편집 명령의 계약은 조회보다 무겁다(파일을 바꾼다). 공식 절차:

```bash
# ① dry-run: 파일을 만들지 않고 무엇이 바뀔지만 본다
rhwp edit fill-fields in.hwp --data @row.json -o out.hwp --dry-run --json

# ② 실채움
rhwp edit fill-fields in.hwp --data @row.json -o out.hwp --json

# ③ 재독 대조: 보고를 믿지 않고 산출물을 다시 읽는다
rhwp fields out.hwp --json | jq -e '[.fields[] | select(.name=="성명")][0].value == "홍길동"'
```

`--dry-run` 은 절대 파일을 만들지 않고, 실패 경로는 출력 파일을 쓰지 않는다
(`edit_fill_fields_contract` 테스트가 고정). `-o` 생략 시 산출물은 **입력 파일 옆**에
`<입력명>_filled.hwp(.hwpx)` 로 생기고, 형식은 입력을 따른다(HWPX 입력 → HWPX 산출).

### 1-4. 채우면 안 되는 필드 — 로고 셀 판별

`기관명` 류 필드가 로고 이미지가 든 셀 안에 있는 서식이 실존한다
([보도자료 사례](../report/edit_demo_hongbo/README.md)). 텍스트를 넣으면 로고와
겹쳐 렌더된다 — rhwp 의 결함이 아니라 서식의 성격이고, 실제 사용자도 그 칸은
로고로 둔다. 판별: `fields --json` 의 `location.nested` 로 어느 셀인지 확인하고,
그 셀에 그림이 있는지 `export-tables` 로 본 뒤 건너뛴다.

## 2. `set-cell` 심화

### 2-1. 좌표는 `export-tables` 와 동일한 격자다

```bash
rhwp export-tables 양식.hwpx --json | jq '.tables[0].cells[] | select(.text=="")'
rhwp edit set-cell 양식.hwpx --table 0 --row 2 --col 1 --text "1,234" -o out.hwpx --json
```

`--table` 은 본문 최상위 표 번호(export-tables 의 index), `--row`/`--col` 은 0 기준.

### 2-2. 병합으로 덮인 칸은 실패가 정답이다

병합(rowSpan/colSpan)으로 덮인 좌표에 쓰면 명령은 **앵커 좌표를 안내하며 실패**한다.
이는 오류가 아니라 보호다 — 사람 눈에 한 칸인 곳의 숨은 좌표에 값을 넣으면
렌더에 안 보이는 유령 데이터가 생긴다. 실패 메시지의 앵커 좌표로 다시 쓴다.

### 2-3. `overflow` — 성공했는데 제출 불가인 산출물 막기

값이 칸 폭을 넘치면 `--json` 응답의 `overflow` 로 보고된다(채우기 자체는 막지 않음).
근사 판정(한글 전각·ASCII 반각)이므로 최종 확인은 렌더로 한다:

```bash
r=$(rhwp edit set-cell in.hwp --table 0 --row 3 --col 2 --text "$V" -o out.hwp --json)
if [ "$(echo "$r" | jq '.overflow | length')" -gt 0 ]; then
  echo "칸 넘침 — 값 축약 필요: $(echo "$r" | jq -c '.overflow')"
fi
```

실측 사례: 54쪽 정부 서식에서 7자 값이 5자 칸에 거부 판정, 5자 값은 통과
([MSS 사업계획서 사례](../report/edit_demo_mss_bizplan/README.md)).
누름틀 축(`fill-fields`)의 같은 보고는 #3480 에서 진행 중이다.

## 3. `replace-text` 심화

```bash
rhwp edit replace-text 공문.hwp --find "2025년" --replace "2026년" -o 개정본.hwp --json
rhwp search 개정본.hwp "2025년" --json | jq -e '.matchCount == 0'   # 잔존 0 확인
```

- 치환 0건이면 **출력 파일을 만들지 않는다** — "아무것도 안 바뀐 사본"이 생기지 않는다.
- 검증은 항상 산출물 재검색으로 닫는다(위 두 번째 줄). `replacedCount` 보고만 믿지 않는다.

## 4. 최종 게이트 — 눈이 아니라 기계로 닫기

어느 축이든 마지막은 같다:

```bash
# ① 내용 무결성: 바꾸려던 것만 바뀌었나
rhwp ir-diff 원본.hwp 산출물.hwp --json | jq -c '{identical, diffCount, categories}'
# 편집했으므로 identical:false 가 정상 — categories 가 의도한 변경(텍스트/셀)만 담겼는지 본다

# ② 시각 확인이 필요한 제출물: 바뀐 페이지만 렌더해 사람/VLM 확인
p=$(rhwp search 산출물.hwp "홍길동" --json | jq '.matches[0].page')
rhwp export-svg 산출물.hwp -p "$p" --json
```

다쪽 서식은 **건드리지 않은 페이지의 불변**까지 확인한다 — 54쪽 서식에서
2쪽만 편집했으면 나머지 52쪽 렌더가 원본과 픽셀 동일해야 한다
(절차는 [MSS 사례](../report/edit_demo_mss_bizplan/README.md)의 재현 스크립트).

## 5. 함정 목록 (실측 근거)

| 함정 | 증상 | 처방 |
|---|---|---|
| `--data` 파일을 cp949 로 저장 | `stream did not contain valid UTF-8` (exit 1) | 데이터 파일은 항상 UTF-8 로 쓴다 (Python 이면 `encoding='utf-8'` 명시) |
| `ambiguous` 무시 | 반복 필드의 첫 칸만 채워진 채 제출 | 1-1 의 `[N]` 재지목 루프 |
| `notFound` 무시 | 오타 필드가 빈 칸으로 제출 | 1-2 의 jq 게이트 |
| 병합 좌표에 set-cell | 실패 + 앵커 안내 | 안내된 앵커 좌표로 재시도 |
| 긴 값 밀어넣기 | overflow 보고 (렌더 겹침/잘림) | 값 축약 또는 서식 재검토 |
| 로고 셀 채움 | 로고와 텍스트 겹침 렌더 | 1-4 판별 후 건너뜀 |
| 보고만 믿음 | 산출물 미검증 | 재독(`fields`/`search`) + `ir-diff` 로 닫기 |

## 실물 사례 (전 과정 기록)

- [보도자료 서식 — 누름틀 11개 + MCP 경로 검증](../report/edit_demo_hongbo/README.md)
- [54쪽 정부 사업계획서 — set-cell + 다쪽 픽셀 불변 검증](../report/edit_demo_mss_bizplan/README.md)
- [복학원서 — 표 격자 좌표 채우기](../report/edit_demo_bokhak/README.md)
