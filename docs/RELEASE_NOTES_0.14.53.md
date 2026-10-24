# cys 0.14.53 — 릴리스 노트

## Windows 스마트 앱 컨트롤(SAC) · Windows Smart App Control

<!-- cys-win-sac: unsigned -->
**Windows 스마트 앱 컨트롤(SAC):** 이 판의 cys 설치 파일과 cys 실행파일은 코드 서명이 없습니다. 스마트 앱 컨트롤이 켜진 PC에서는 실행이 막힐 수 있으며, 이 판은 스마트 앱 컨트롤 적용 PC에서 검증하지 않았습니다.
**Windows Smart App Control:** this release's cys installer and cys executables are not code-signed. They may be blocked on PCs with Smart App Control on, and this release was not verified on a Smart App Control PC.

- 이 문단은 릴리스 본문과 앱 업데이트 정보(`latest.json` notes)에도 그대로 실린다. 공개 직전 검사(`scripts/release-verify.py` 8단계)가 이 고지와 설치 파일의 실제 서명 상태가 맞는지 확인하고, 다르면 공개를 막는다.
- 코드 서명은 오너의 서명 자격증명(조직 대상 Microsoft Artifact Signing 또는 신뢰 CA 인증서)이 마련되면 별도 설계로 켠다. 이 판에서 서명 모드는 `unsigned` 하나만 허용된다.
