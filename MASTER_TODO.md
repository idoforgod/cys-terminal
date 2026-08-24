# master TODO — 셸에 cys 설치 복원 + 수리 (2026-08-25)

레인: /Users/cys-macbook/dev/cys-terminal-fix @ fix/shell-cli-install-restore (base c54c3b2 = v0.14.24)
★금지: /Users/cys-macbook/dev/cys-terminal-rel (타 세션 캠페인) · 상대 상위 충돌파일 8종

## Scope A — 복원 + 수리 (지금 진행 · 상대 "안전 영역"만 사용)
- [ ] A1 버튼 복원 (ui/index.html cc-header · id=btn-install-cli)
- [ ] A2 수리1 플랫폼 게이팅 — macOS 외 미노출 (Rust Err는 심층방어로 존치)
- [ ] A3 수리2 등급 분리 — status: installed / installed_shadowed / unverified (측정불능≠통과)
- [ ] A4 수리3 해제 경로 — uninstall_cli_from_path + cli_install_status (심링크만·실체파일 보호)
- [ ] A5 선택4 NonStandard 번들 거부 승격
- [ ] A6 선택5 which -a 타임아웃 (측정 실패 → unverified)
- [ ] A7 단위테스트 신규 + 기존 39 유지
- [ ] A8 문서 정합 (INSTALL §B · USER-MANUAL §2.4 · INST-DENY-02)

## Scope B — PYTHONDONTWRITEBYTECODE (보류: 상대 충돌 상위파일)
- [ ] B1 현행 실태 실측
- [ ] B2 상대 태그 후 rebase하여 구현

## Scope C — Gatekeeper 격리 재현 릴리스 게이트
- [ ] C1 기존 게이트(4f14e04) 실재·커버리지 실측
- [ ] C2 격차분 보강

## Scope D — 릴리스 (★상대 태그 이후에만)
- [ ] D1 상대 태그 대기·rebase
- [ ] D2 버전 SOT 범프 → CI → DMG 2종 공증 → 윈도우 setup/zip
- [ ] D3 WDSI 1회 시도
- [ ] D4 홈페이지 downloads 업로드 + SHA256SUMS 전체 갱신 + Defender 안내 섹션 잔존 검증

## 성찰 (구현 후)
- [ ] R1 설계안 정확 이해 여부
- [ ] R2 적대적 검증 · 기준=최고 품질
- [ ] R3 30년차 아키텍트 — 의존성/결합도/파급
