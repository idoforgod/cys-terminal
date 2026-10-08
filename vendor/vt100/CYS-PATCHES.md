# vendor/vt100 — cys 로컬 패치 대장

이 폴더는 상류 `vt100` 0.15.2(doy/vt100-rust · MIT)를 들여와 **고쳐 쓰는 사본**이다(`Cargo.toml` 의 벤더링 주석 · `NOTICE.md`).
상류 새 판으로 덮으면 아래 패치가 사라진다. 자리마다 소스에 `cys 패치` 주석이 있다.

| 커밋 | 무엇 | 자리 | 지키는 시험 |
|---|---|---|---|
| 1f6d899c (2026-10-07) | 상류 0.15.2 를 원본 그대로 들여옴 | — | — |
| 1e758593 (2026-10-07) | 화면 파서 패닉 수리 — 폭 축소 뒤 짝 없는 넓은 글자 머리 · 범위 밖 짝 칸 · u16 언더플로 · 축소 뒤 DECRC | `grid.rs` · `row.rs` · `screen.rs` 의 `cys 패치` 주석 14곳 | `src/bin/cysd/state.rs` 의 `row89_sequence_no_longer_panics_and_screen_is_preserved` · `shrink_clears_orphan_wide_head_down_to_one_col` |
| ffa95a52 (2026-10-07) | 벤더 `Cargo.toml` 의 상류 저자 이메일 제거 | `Cargo.toml` | — |
| 0.14.47 | **흐림(SGR 2) 비트** — SGR 2 를 칸에 기억하고 22·0 으로 내린다 · `Cell::dim()` 으로 읽는다 · **칸 비교와 다시 그리기 출력에는 넣지 않는다** | `attrs.rs`(비트 · `dim`/`set_dim` · 흐림을 가린 `PartialEq`) · `screen.rs`(SGR 2 · 22 팔) · `cell.rs`(`dim()`) | `src/bin/cysd/vt100_dim_tests.rs` 의 다섯 시험 |

## 흐림 패치의 결정 (0.14.47)

- 흐림은 **읽기 전용 재료**다. 읽는 쪽은 큐 막힘 진단의 화면 탐침 하나(`screen_diag.after_cursor_dim`)다.
- `Attrs` 의 비교(`==`)는 흐림 비트를 가리고 본다. 그래서 `contents_formatted()` · `contents_diff()` 의 바이트는 패치 전과 같다
  (attach 로 다시 붙을 때 데몬이 그려 주는 첫 덩어리가 바뀌지 않는다). `vt100_dim_not_in_formatted_output` 이 패치 전에 떠 둔 고정값으로 지킨다.
- SGR 22 는 터미널 규약대로 굵게와 흐림을 **함께** 내린다.

## 상류를 새 판으로 올릴 때

1. 올리기 전에 위 표를 읽는다.
2. 들여온 뒤 표의 커밋을 차례로 다시 얹는다(충돌하면 「자리」 설명을 보고 손으로).
3. 표의 시험이 전부 녹색이어야 한다. `vt100_dim_tests.rs` 가 컴파일되지 않으면(`dim()` 없음) 흐림 패치가 빠진 것이다.
4. 상류가 같은 기능을 넣었으면(예: 상류가 흐림을 지원) 로컬 패치를 빼고 그 행을 「상류 흡수」로 고친다.
   단 상류가 흐림을 **비교·출력에 넣는다면** `vt100_dim_not_in_formatted_output` 이 죽는다 — 그때는 attach 바이트가 바뀌는 것을 받아들일지 따로 정한다.
