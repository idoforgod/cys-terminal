# 표준 런북 — cys-terminal 정식 릴리스 (vX.Y.Z)

> **v0.13.3에서 전 구간 실전 검증된 확정판** (2026-07-24 완주: 10차 CI 사이클 → draft → 공개 →
> cysinsight.com 업로드 → `remote release verified: assets=13`). 차기 릴리스는 이 문서를
> 위에서부터 실행하면 된다. 값이 필요한 자격증명은 전부 격리 파일 참조(값 재기재 금지).
> 레포: `_worktrees/cys-terminal-browser-v2` (github.com/idoforgod/cys-terminal)

## 0. 전제 (1회 설정 — 이미 완료된 상태, 새 머신에서만 재확인)

| 항목 | 위치 | 비고 |
|---|---|---|
| GitHub PAT | osxkeychain(username=idoforgod) | **권한 3종 필수: Contents RW + Actions RW + Workflows RW**. 중복 keychain 항목 주의(구 항목이 잡히면 403 — `security delete-internet-password -s github.com` 반복 후 재저장) |
| Apple 공증 | `~/.cys/apple-notary.env`(600) | 서명·공증은 CI secrets 소관. 401 나오면 **먼저 로컬 `xcrun notarytool history`로 앱 암호 유효성 판정**(무효면 account.apple.com→앱 암호 재발급→GitHub 시크릿 `APPLE_PASSWORD` 갱신) |
| Hostinger | `~/.cys/hostinger-ftp.env`(600) | FTP+SFTP 동일 계정(포트·호스트는 env 파일). gitignore 영역 — 레포 접촉 금지 |
| Windows 무서명 | repo variable `ALLOW_UNSIGNED_WINDOWS=1` | 설정돼 있음(지속). 인증서 도입 시 변수 삭제+시크릿 3종 설정 |
| 환경 보호 | release-production Deployment branches | 릴리스 브랜치가 목록에 있어야 publish 가능 |

## 1. 코드 준비 → 태그

```bash
cd /Users/cys-macbook/Desktop/CYSjavis/_worktrees/cys-terminal-browser-v2
# 버전 SOT 6곳: Cargo.toml, src-tauri/Cargo.toml, src-tauri/tauri.conf.json,
#              ui/package.json, dist-win/cys.wxs, dist-win/cys-x64.wxs (+Cargo.lock 자체 패키지 2곳)
bash scripts/version-check.sh vX.Y.Z          # 6곳 일치 확인
# 로컬 사전 게이트(CI 낭비 방지 — 전부 green 후 태그):
sh -n cysjavis-pack/hooks/*.sh && python3 cysjavis-pack/bin/javis_bootstrap.py --self-test
python3 -m unittest discover -s scripts/tests -q     # (로컬 py3.9의 test_release_assemble 1 error는 기준선)
bash scripts/secret-scan.sh --all && bash scripts/scan-pack-secrets.sh
shellcheck scripts/build-macos-signed.sh scripts/import-macos-signing-certificate.sh \
  scripts/make-update-manifest.sh scripts/release-handoff.sh \
  scripts/runtime-stage-sign-macos.sh scripts/verify-release-remote.sh   # CI 동일 6파일(바이너리는 스크래치 설치)
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"  # 워크플로우 수정했다면
# main의 릴리스 인프라 수정 선이식 확인(AppleDouble 사례): git log <분기점>..github/main --oneline -- scripts/
git push github <브랜치>
git tag -a vX.Y.Z -m "vX.Y.Z — <요지>" <sha> && git push github vX.Y.Z
```
재태그(사이클 반복) 시: `git push github :refs/tags/vX.Y.Z && git tag -d vX.Y.Z` 후 재생성.
⚠ push는 pack-objects로 수 분 걸릴 수 있음(background 권장). git diff/show --stat은 이 트리에서 hang.

## 2. CI 감시 (태그 push → release.yml 자동)

잡 체인: release-contracts → build×3(macOS 2종은 서명·공증·staple까지) → pack-artifacts(서명 pack) →
assemble-draft(14자산+SHA256SUMS+사이트번들+**draft**). API 폴링:
`GET /repos/idoforgod/cys-terminal/actions/runs?per_page=1` → jobs. 실패 시 잡 로그로 진단
(함정 목록은 메모리 `cys-release-standard-procedure` 참조 — shellcheck·스캐너 2종·minisign·
cargo placeholder·self-held fd·AppleDouble 등 10건 기소거).

## 3. draft 검증 (로컬)

```bash
SP=<스크래치>/release-vX.Y.Z; mkdir -p $SP
# 자산 14종 다운로드: GET /releases → assets[].id → GET /releases/assets/{id} (Accept: octet-stream)
( cd $SP && shasum -a 256 -c SHA256SUMS.txt )                     # 13종 전부 OK
python3 scripts/release-verify.py --version X.Y.Z --release-dir $SP   # 디렉토리에 14파일만 둘 것
xcrun stapler validate $SP/cys_X.Y.Z_{aarch64,x64}.dmg            # 공증 staple
hdiutil attach $SP/cys_X.Y.Z_aarch64.dmg -nobrowse -mountpoint /tmp/d \
  && spctl -a -vv /tmp/d/cys.app && hdiutil detach /tmp/d          # "accepted · Notarized Developer ID"
```

## 4. 공개 승격 (API — 사람 클릭 0)

```bash
BUNDLE_SHA=$(shasum -a 256 $SP/SHA256SUMS.txt | awk '{print $1}')
POST /actions/workflows/release-publish.yml/dispatches
  {"ref":"<브랜치>","inputs":{"tag":"vX.Y.Z","release_bundle_sha256":"$BUNDLE_SHA","confirm":"PUBLISH"}}
# run이 waiting이면: GET /actions/runs/{id}/pending_deployments → environment id 획득 →
POST /actions/runs/{id}/pending_deployments {"environment_ids":[<id>],"state":"approved","comment":"..."}
# 완료 후 릴리스 draft=false·latest 확인
```

## 5. 사이트 번들 재조립 (CI와 byte-identical — 결정론 실증됨)

```bash
PUB=$(python3 -c "import json;print(json.load(open('$SP/latest.json'))['pub_date'])")
# assets-policy.json의 input_assets 11종만 IN 디렉토리로 복사 후:
python3 scripts/release-assemble.py --version X.Y.Z --input IN --output OUT \
  --repository idoforgod/cys-terminal --pub-date "$PUB" --site-output SITE
# OUT의 zip/SUMS/latest가 draft와 IDENTICAL인지 대조(결정론 확인) — 페이지는 SITE/downloads/
```

## 6. 웹 업로드 (★순서 고정: 자산 → SHA256SUMS → 페이지)

- **경합 방지 규약**: `/downloads/`의 pack 3종은 라이브 pack-update 채널 — **기록 전 편대(pane master)에 cys send로 통지**. 사이트 SUMS와 pack 3종은 원자 세트(단독 선발행 금지).
- 전송: 대형 자산은 FTPS(`curl --ssl-reqd -k -T <f> ftp://$FTP_HOST/downloads/<f>`; 인증서 CN=*.hstgr.io라 -k 필요 — 무결성은 7단계 해시검증이 백스톱). **해시밀도 파일(pack-manifest.json 등)이 450 "Link to file server lost"로 거부되면 SFTP로**:
```bash
sftp -P <SSH포트> <hostinger-계정>@<hostinger-호스트>   # 암호 자동화는 /usr/bin/expect (대시 명령은 send --)
put <파일> domains/cysinsight.com/public_html/downloads/<파일>
```
- ⚠ zsh에서 curl 명령을 변수에 담지 말 것(단어 미분할). 청크 append 우회는 금지(부분 실패 시 원격 오염 — 실사고).

## 7. 원격 검증 (완료의 정의)

```bash
bash scripts/verify-release-remote.sh --version X.Y.Z \
  --release-dir <14파일 클린 디렉토리> --site-root https://www.cysinsight.com
# → "remote release verified: version=X.Y.Z assets=13" 이 나와야 완료.
# 이 스크립트가 ⓐDefender 마커+문구(+xattr 금지) ⓑSUMS byte동일+전자산 해시 ⓒrecovery 마커를 전부 fail-closed 검증.
# ⓒ공증은 3단계에서 선검증(원격 자산=동일 바이트). ⓓ콜드스타트 실기: 새 pane→claude→"너는 마스터다"→1차 5노드.
```

## 8. 사후

- 릴리스 사용 자격 로테이션 권장(채팅 노출분): FTP 비밀번호·GitHub PAT.
- 환경 보호에 임시 추가한 배포 브랜치 제거(선택).
- SESSION_STATE·메모리에 버전·특이사항 기록. 이 런북과 어긋난 것은 런북을 갱신(문서가 SOT).
