# 교훈 대장 — v0.13.3 릴리스 시행착오 전체 기록 (재발 방지 결정판)

> 2026-07-24 · 10차 CI 사이클로 완주한 v0.13.3의 **모든 실패·원인·수정·예방 게이트**.
> 목적: 차기 릴리스에서 같은 시행착오 0회. 실행 절차는 `RUNBOOK-release-STANDARD.md`,
> 요약 색인은 메모리 `cys-release-standard-procedure` — 이 문서는 상세 근거 대장이다.

## A. CI 결함 10건 — 원인·수정·예방

| # | 차수 | 증상 | 근본 원인 | 수정(커밋) | ★예방 게이트(로컬에서 잡는 법) |
|---|---|---|---|---|---|
| 1 | 1차 | release-contracts 실패 | verify-release-remote.sh SC2015(`A&&B\|\|C`) — 이 파일의 첫 릴리스 CI 통과 시도 | d56d083 | 태그 전 `shellcheck <6파일>`(공식 바이너리 스크래치 설치) — 런북 1단계 |
| 2 | 2차 | 3-OS 전부 secret-scan 차단 | self-test 픽스처 `/Users/x\n` — 더미 면제는 후행 `/`·`"`·줄끝만 | 0fe1ecc(→/tmp/x) | `bash scripts/secret-scan.sh --all` 태그 전 실행 |
| 3 | 3차 | minisign 설치 스텝 exit 2 | `minisign --version` — minisign은 롱 옵션 없음(`-v`만) | fb181f5 | 새 CLI 도입 시 플래그를 로컬 실행으로 검증 |
| 4 | 3차 | browserd 테스트 Windows 실패 | 경로 기대값 POSIX 리터럴(`\home` vs `/home`) | df45b82 | 경로 기대값은 구현과 동일한 `node:path.join`으로 조립 |
| 5 | 4차 | aarch64 cargo test 사망 | `cargo test -p cys-app`이 browser-runtime 스테이징보다 먼저 → tauri build.rs 리소스 존재검증 실패 | b39af4f(`mkdir -p` placeholder — 스테이징은 빈 디렉토리만 원자 대체함을 코드로 증명) | 워크플로우에서 "리소스 요구 시점 < 리소스 생성 시점" 순서 감사 |
| 6 | 4차 | Windows WinError 32 | (1차 오진: AV 잠금 → 재시도 백오프 17e86d9) | — | — |
| 7 | 5차 | 같은 지점 재실패(56초 소진) | **진범 = self-held fd**: NamedTemporaryFile with-블록 안에서 rename(POSIX는 허용·Windows는 불가) | b475107(핸들 close 후 교체 + AST 봉인 테스트) | **WinError 32는 AV 의심 전에 자기 핸들부터** — with-블록 안 rename 금지 |
| 8 | 4~5차 | 공증 HTTP 401 | CI secret의 앱 암호가 애플측 무효(Apple ID 비번 변경 시 전체 무효화) | 신규 발급+시크릿 갱신 | 401이면 시크릿 오타 의심 전에 **로컬 `xcrun notarytool history`로 선판정**(40분 빌드 낭비 차단) |
| 9 | 6차 | Windows credential contract 차단 | 브랜치 신설 fail-closed 게이트 vs 오너는 Authenticode 미보유(무서명+Defender 안내가 공식 모델) | b509404(`ALLOW_UNSIGNED_WINDOWS` 명시 옵트아웃·updater .sig는 유지·자산 14종 불변) | repo variable 1회 설정으로 지속 — 재설정 불요 |
| 10 | 7차 | metadata 서명 "Unsupported key derivation function" | choco minisign 구버전이 무암호 키 KDF 미지원(brew 0.12는 성공) | 9512e10(공식 0.12 win64 zip 다이제스트 고정 설치) | 크로스 OS 도구는 **버전 동기**가 계약 — 설치 채널 불일치 금지 |
| 11 | 8차 | scan-pack-secrets 4건 차단 | browserd 테스트 `/home/user` — pack 발행 스캐너는 빌드 3-OS green 후에야 처음 도는 **늦은 게이트** | 3d0858e(`/Users/x` — 두 스캐너 동시 통과 형식) | `bash scripts/scan-pack-secrets.sh` 태그 전 실행 |
| 12 | 9차 | assemble "extra=`._*.dmg`" | macOS tar의 AppleDouble 사이드카 — main엔 수정 존재(PR#31)·브랜치가 미상속 | 2ab9ee5(상류 blob과 byte 동일 이식: `COPYFILE_DISABLE=1`) | 릴리스 브랜치는 main의 릴리스 인프라 수정 선이식 확인(`git log 분기점..github/main -- scripts/`) |

## B. 자격·인프라 관문 — 실측 확정 사실

1. **GitHub PAT 3권한**: Contents RW(코드 push) + Actions RW(디스패치·승인 API) + **Workflows RW**(.github/workflows 수정 커밋 push — Actions와 별개 항목, 없으면 push 자체가 거부). Fine-grained 기본값은 read-only/Public — 만들고 나서 Repository access·Permissions 저장(Update token) 확인 필수.
2. **keychain 중복 함정**: 항목이 2개면 git이 구(무효) 항목을 집어 403 — API로는 되는데 git만 실패하는 미스터리의 정체. `security delete-internet-password -s github.com` 반복 → 단일 재저장.
3. **토큰 쓰기 검증법**: 저장소 API의 permissions 필드는 사용자 권한이지 토큰 범위가 아님 — blob 생성 API(201)가 진짜 판정.
4. **환경 보호 2중**: release-production은 ①배포 브랜치 제한(릴리스 브랜치를 목록에 추가) ②승인 게이트(`pending_deployments` API로 오너 토큰 승인 — UI 클릭 불요).
5. **Hostinger 전송**: FTP(21·FTPS·인증서 CN=`*.hstgr.io`라 `-k` 필요)로 대형 자산 OK. **해시밀도 파일(pack-manifest.json)은 450 "Link to file server lost"로 콘텐츠 차단** — 정공법은 SFTP 포트 <SSH포트>(같은 계정·docroot=`<docroot — hPanel 확인>/`), macOS 암호 자동화는 `/usr/bin/expect`(sftp 대시 명령은 `send --`). **청크 append 우회 금지**(부분 실패 시 원격 오염 실사고).
6. **이 맥의 한계**: 로컬 서명 불가(Developer ID 인증서·`~/.tauri/cys-updater.key` 부재 — 구 머신 소재), gh CLI·brew 부재, git diff/show --stat hang, push는 pack-objects 수 분.

## C. 프로세스 교훈 — 시행착오의 구조적 원인과 처방

1. **"브랜치의 첫 릴리스 CI 완주"는 잠재 결함의 저수지다** — 이번 10건 중 9건이 우리 코드가 아니라 브랜치에 잠재하던 것. 처방: 태그 전 **로컬 사전 게이트 풀세트**(런북 1단계: version-check·sh -n·self-test·unittest·secret-scan·scan-pack-secrets·shellcheck·YAML 파싱)를 돌리면 사이클 1·2·8은 0비용으로 예방된다. CI에서만 잡히는 것(OS별 러너 동작·secrets)은 잡 로그 API 즉시 회수(`/actions/jobs/{id}/logs`)로 사이클당 진단 시간을 분 단위로.
2. **늦은 게이트 지도**: contracts(3분) → build(20-40분) → pack-artifacts → assemble 순으로 게이트가 늦게 나타난다. 뒤 단계 게이트(scan-pack-secrets·자산 집합 검증)는 로컬 대응물을 먼저 돌려라.
3. **오진 비용**: WinError 32를 AV로 오진해 사이클 1개를 소모했다. 증거(56초 소진)가 가설(일시 잠금)과 모순되면 즉시 소스로 내려가라 — 실행 문맥("이 코드는 어느 프로세스·어느 핸들 상태에서 도는가")이 1번 질문.
4. **공유 프로덕션 채널 경합(실사고)**: `/downloads/`의 pack 3종은 라이브 pack-update 채널이라 다른 조직(편대 pane master)이 동시 기록 가능 — 실제로 업로드 4분 뒤 0.13.6 트리오가 우리 것을 덮었다. **규약: 사이트 SUMS와 pack 3종은 원자 세트(단독 선발행 금지)·기록 전 조직 간 cys send 상호 통지.** 경합 발견 시 싸우지 말고 상대 화면 실측(read-screen) → 양측 합의 상태 확인 후 복원.
5. **자격은 선검증이 철칙**: 무효 앱 암호로 빌드 2사이클(~80분)을 태웠다. 모든 외부 자격(공증·PAT·FTP)은 사용 전 무해한 read 호출로 유효성 판정.
6. **사이클 경제**: 수정 여러 건이 모이면 **한 번의 재태그로 묶어라**(태그 삭제·재생성은 무비용, CI 40분이 진짜 비용). 단 검증 안 된 수정을 몰아넣지는 말 것 — 이번엔 매 수정을 로컬 게이트 green 후에만 합류시켰다.

## D. 재발 방지 체크(차기 릴리스 시작 시 1분 점검)

- [ ] 런북 0장 전제 5항목 생존 확인(PAT 3권한·notary env·hostinger env·ALLOW_UNSIGNED_WINDOWS·환경 브랜치)
- [ ] `xcrun notarytool history` 앱 암호 선검증
- [ ] 로컬 사전 게이트 풀세트(런북 1단계) 전부 green 후에만 태그
- [ ] main의 릴리스 인프라 수정 선이식 확인
- [ ] `/downloads/` 기록 전 편대 통지
