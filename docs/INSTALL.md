# cys 터미널 설치 가이드

> cys는 **AI 조직(자비스)이 거주하는 터미널**입니다. 코어 데몬(cysd) + CLI(cys) + GUI 앱(cys.app)으로
> 구성되며, **받는 사람은 데몬을 따로 설치할 필요가 없습니다** — 앱·CLI가 자동으로 기동합니다.

> **이 문서는 두 종류의 독자를 위해 명시 분리되어 있습니다.**
> - 🧑 **[HUMAN]** 표시 = **사람이 직접** 해야 하는 단계(DMG 더블클릭·Gatekeeper·`sudo`). 에이전트는 자율 실행하지 말고 사람에게 위임합니다.
> - 🤖 **[AGENT]** 표시 = 에이전트(워커)가 부트 시 **결정론으로 따르는** 실행 스텝(`cys boot`/`launch-agent` 등).
> - 🚧 **[BOUNDARY]** 표시 = 설치 중 **무조건 정지·오너 보고** 대상인 위험 명령(아래 "설치 경계" 섹션 참조).
>
> 한 줄이 사람용 복붙인지 에이전트 실행 스텝인지가 헷갈리면, 이 표시를 우선합니다.

## 무엇이 자동인가 (0.2.0+)

신규 머신에서 별도 설정 없이 동작합니다:

1. **데몬 자동 기동** — 앱을 열거나 `cys` 명령을 처음 쓰면, 데몬이 없을 때 번들에 동봉된 `cysd`를
   분리(detached) 프로세스로 자동 기동합니다. (옵트아웃: `CYS_NO_AUTOSTART=1`)
2. **Pack 자동 설치** — 데몬 첫 기동 시 `~/.cys/pack`(디렉티브·스킬·스케줄·ACL)을 자동 설치합니다
   (보존 모드 — 기존 파일은 덮어쓰지 않음).
3. **pane 내 PATH 주입** — 데몬이 만드는 모든 pane의 `PATH` 선두에 `cys` 바이너리 폴더가 들어가,
   pane 안의 AI가 `cys identify`를 바로 쓸 수 있습니다(심링크 수동 생성 불요).
4. **오프라인 자기완결 pack** — DMG의 `cys.app` 안에는 ⓐ `cys`/`cysd` 바이너리에 pack 전 트리가
   임베드되고, ⓑ 빌드 시(`bundle-prep.sh`) 동일성·콘텐츠 스캔 게이트를 통과한 **단일 `pack.tar.gz`
   (+`pack-manifest.json`)** 가 `Contents/Resources/` 에 동봉됩니다. 따라서 네트워크 없이도 첫 기동에
   pack을 설치할 수 있고(자기완결), 동봉본은 가시적이라 검수·핫스왑이 가능합니다. 동봉 대상은 raw
   스킬 트리가 아니라 **`cys pack-manifest`(임베드 권위 SOT)가 지정한 파일집합만으로 만든 결정론 단일
   tar blob** 입니다 — 개인정보·미추적 쓰레기 파일 박제를 구조적으로 회피합니다.

→ 따라서 GUI만 쓰는 사용자는 **앱을 더블클릭하면 끝**입니다.

## 🚧 설치 경계 (Boundaries — 무조건 정지·오너 보고)

> 이 섹션은 **에이전트(워커)용 1급 경계**입니다. 아래 명령들은 부트·설치 중에 워커가 자율로 밟지 말고,
> 만나면 **무조건 중단하고 오너에게 보고**합니다. 거버넌스 일반 denylist(외부 발행·비가역 삭제는
> 무조건 중단·보고)는 이미 워커 절대지침으로 주입되며(근거: `directives/WORKER_DIRECTIVE.md §7`),
> 이 섹션은 **설치에 특화된 위험 명령**만 별도 열거해 보강합니다(directive가 담지 않는 부분).

| ID | 위험 명령 | 왜 경계인가 | 워커 처리 |
|---|---|---|---|
| INST-DENY-01 | `cys daemon install --takeover` | **가동 중인 세션이 소멸**합니다(비가역 — 아래 "C. 상시 가동" line 참조). | 자율 실행 금지 → 정지·오너 보고 |
| INST-DENY-02 | `sudo ln -sf …` (심링크 덮어쓰기) | `sudo` = 오너 권한 단계 + `-f`로 기존 파일을 묻지 않고 덮어씀. | 사람(🧑)이 직접 실행 → 워커는 위임. **GUI "셸에 cys 설치"/"셸 cys 해제" 버튼은 사용자 명시 클릭 + osascript 1회 승격이라 이 경계를 위반하지 않지만, 에이전트가 그 버튼을 자율로 클릭하는 것은 여전히 금지**(사람의 클릭이 곧 동의다 — 대리 클릭은 동의가 아님). |
| INST-DENY-03 | `rm -rf ~/.cys ~/.local/state/cys` | pack·트랜스크립트·상태 **완전 삭제**(비가역). | 자율 실행 금지 → 정지·오너 보고 |
| INST-DENY-04 | DMG 우클릭·Gatekeeper "열기"·코드사이닝 | 사람 GUI/보안 결정 단계. | 사람(🧑)이 직접 → 워커는 위임 |

> 위 경계는 거버넌스 denylist의 **install 특화 투영**입니다(새 denylist 생성 아님). 일반 경계는
> `WORKER_DIRECTIVE.md §7`이 정본이며 여기서 재서술하지 않습니다 — 충돌·중복 표현형 방지.

## 선행조건 — git (기능별 필수)

기본 사용(DMG/MSI 설치 → 앱 더블클릭)에는 git이 **필요 없습니다**. 다만 아래 기능을 쓰려면 git이 있어야 합니다:

- **소스 기여·직접 빌드**: `git clone https://github.com/idoforgod/cys-terminal`
- **harness-creator 툴체인 자동 설치**: 부트 프리플라이트(`javis_preflight.py --fix`)가 git으로 자동 클론합니다.
- **RSI 자기개선/자동 발행**: 라운드 산출물의 로컬 커밋·외부 push에 git을 씁니다.

프리플라이트가 git 유무를 결정론으로 점검합니다(`C30.git` — 부재 시 WARN으로 안내). 설치:

```bash
# 🧑 [HUMAN] macOS — Xcode Command Line Tools(권장) 또는 Homebrew
xcode-select --install        # 또는: brew install git
# 🧑 [HUMAN] Linux
sudo apt install git          # Debian/Ubuntu
sudo dnf install git          # Fedora/RHEL
# 🧑 [HUMAN] Windows — 공식 인스톨러
#   https://git-scm.com/download/win  (설치 후 새 터미널에서 git --version)
git --version                 # 설치 확인 (사람·에이전트 공통)
```

## macOS

### 🧑 A. 설치파일 (DMG) — 권장 [HUMAN — 사람이 직접]
1. `cys-0.2.0-macos-arm64.dmg`를 열고 `cys.app`을 `Applications`로 드래그.
2. 첫 실행 시 Gatekeeper 경고가 나오면: 우클릭 → "열기"(미서명 빌드의 경우). 🚧 [BOUNDARY INST-DENY-04 — 사람 보안 결정]
   서명·공증된 빌드라면 바로 열립니다.
3. 끝. 앱이 데몬을 자동 기동합니다.

#### 받는 분(비기술자)께 — 1페이지 (D6 제품 모드 패키징)
> 터미널을 한 번도 안 열어도 오너 대표 산출물(통찰보고서·문체 글)을 클릭으로 받을 수 있습니다.
1. `cys.app`을 `Applications`로 드래그(설치 끝).
2. 더블클릭 → 열림(공증 빌드면 경고 없음). **"손상되었기 때문에 열 수 없습니다"가 뜨면 아래 [macOS "손상되었기 때문에 열 수 없습니다" 해결](#macos-손상되었기-때문에-열-수-없습니다-해결)을 그대로 따라 하세요** — 원인이 두 가지(미공증 / 반쪽 설치)이고 처방이 다릅니다.
3. 우상단 **Control Center → "스킬 보드"** 탭 → "통찰보고서 만들기" 클릭 → 본문 붙여넣기 → 미리보기 확인 → PDF를 받습니다.
4. ⚠ 산출물은 **"AI 보조 생성 · 오너 검수 전"**입니다. 외부 공유 전 반드시 검수를 받으세요(과대약속 금지).
5. **청중 맞춤**: `~/.cys/profile.json`의 `audience`를 바꾸면(예: `pastor`·`student`) 그 청중에 맞춰 산출됩니다(기본 `custom`=전체보기).

### macOS "손상되었기 때문에 열 수 없습니다" 해결

> 이 메시지 하나에 **원인이 두 가지**입니다. 순서대로 확인하세요. 어느 쪽이든 **설정·대화기록은
> `~/.cys` 에 있어 앱 번들 밖이라, 앱을 지웠다 다시 깔아도 지워지지 않습니다.**

#### 원인 ① 반쪽 설치 (가장 흔함 · 덮어쓰기로 설치한 경우)

이미 설치된 `cys.app` **위에 덮어쓰기**로 새 버전을 설치하면, macOS의 앱 보호 기능(App Management)이
번들 안 일부 파일의 교체만 막습니다. 그런데 복사는 거기서 멈추지 않고 나머지 파일을 전부 바꿔 버려서,
**새 파일과 옛 파일이 섞인 반쪽 앱**이 남습니다. 이 상태에서는 macOS가 앱으로 인식조차 못 해
"손상되었기 때문에 열 수 없습니다"로 차단합니다. (앱이 스스로 이 상태를 감지하면 실행 시
*"설치본이 온전하지 않습니다 — 재설치 필요"* 안내를 띄웁니다.)

**해결 — 덮어쓰지 말고 지운 뒤 새로 설치합니다.**

1. 실행 중인 cys를 완전히 종료합니다.
2. 최신 설치파일(DMG)을 내려받아 엽니다 — <https://www.cysinsight.com>
3. **응용 프로그램 폴더의 기존 `cys.app`을 먼저 휴지통으로 옮깁니다.** ← 이 단계를 건너뛰지 마세요.
4. DMG 안의 `cys.app`을 `응용 프로그램(Applications)` 폴더로 드래그합니다.
5. 응용 프로그램 폴더의 `cys.app`을 엽니다.

> ⚠ **덮어쓰기('바꾸기') 금지** — 3번을 건너뛰고 드래그해서 "바꾸기"를 누르면 같은 고장이
> 그대로 재발합니다. 반드시 **지운 뒤 새로** 넣으세요.
>
> ⚠ **번들 안의 파일만 골라 다시 넣는 '부분 수리'는 통하지 않습니다.** macOS 보호에 막히고,
> 막히지 않더라도 코드서명 봉인이 깨진 채로 남습니다. 유일한 복구는 **통째 재설치**입니다.

#### 원인 ② 인터넷에서 받은 파일의 격리(quarantine) 표시

정상 번들인데도 브라우저로 받은 사본이라 차단되는 경우입니다. 위 1~5를 했는데도 열리지 않으면
터미널에서 한 번 실행한 뒤 다시 여세요:

```bash
xattr -d com.apple.quarantine /Applications/cys.app
```

그래도 안 되면 그 빌드가 **공증(notarization)되지 않은** 것일 수 있습니다. 보낸 사람에게
**"공증 빌드"**를 요청하세요(미공증 빌드는 다른 맥에서 차단됩니다 — `RELEASE.md §1 ★Apple 서명·공증`).

#### 진단 — 무엇이 잘못됐는지 직접 확인

CLI를 쓸 수 있다면 아래가 원인을 이름으로 알려줍니다:

```bash
cys doctor            # app-seal 항목: 코드서명 봉인이 깨졌는지 + 어떤 파일 때문인지
codesign --verify --strict --verbose /Applications/cys.app
```

`bundle format unrecognized, invalid, or unsuitable` 이 나오면 **원인 ①(반쪽 설치)** 입니다.

---

#### 📄 홈페이지 다운로드 페이지용 축약 문구 (초안 — 배포 시 이 블록만 옮기면 됩니다)

> **설치가 안 되나요? "손상되었기 때문에 열 수 없습니다"가 뜬다면**
>
> 이전 버전 위에 **덮어쓰기**로 설치하면 macOS 보호 기능 때문에 파일이 섞인 '반쪽 앱'이 만들어질 수
> 있습니다. 아래 순서로 다시 설치해 주세요. **설정과 대화기록은 지워지지 않습니다.**
>
> 1. 실행 중인 cys를 완전히 종료합니다.
> 2. **응용 프로그램 폴더의 기존 cys.app을 휴지통으로 옮깁니다.** (덮어쓰기 ✗ · 먼저 지우기 ✓)
> 3. 내려받은 DMG를 열고 cys.app을 응용 프로그램 폴더로 드래그합니다.
> 4. 응용 프로그램 폴더의 cys.app을 엽니다.
>
> 그래도 열리지 않으면 터미널에 `xattr -d com.apple.quarantine /Applications/cys.app` 을 한 번
> 입력한 뒤 다시 열어 보세요.

### 🧑 B. CLI도 외부 터미널에서 쓰려면 (선택 · macOS)
앱 번들 안의 cys·cysd를 PATH(`/usr/local/bin`)에 노출합니다. **어느 방법이든 사람이 직접 —
에이전트 자율 실행·자율 클릭 금지.**

1. **권장 — GUI 1클릭(1회 관리자 승인):** Control Center 헤더 → **"셸에 cys 설치"** 클릭 →
   macOS 비밀번호 1회 입력. `/usr/local/bin/cys`·`/usr/local/bin/cysd` 심볼릭이 생깁니다.
   버튼은 macOS에서만 나타납니다(Windows·Linux에서는 표시되지 않습니다).
   - 클릭 뒤 결과 알림이 **성공/경고 두 등급**으로 옵니다. 다음 세 경우는 **아직 끝난 게 아닙니다**
     (경고 알림은 60초 동안 떠 있고, 그 뒤에도 Control Center의 **알람 탭**에서 다시 볼 수 있습니다):
     - **"다른 cys가 앞을 가립니다"** — 심링크는 생겼지만 로그인 셸 기준으로 PATH 앞쪽의 다른
       cys(예: 다른 도구가 설치한 사본)가 먼저 잡힙니다. 알림에 적힌 그 경로를 지우거나
       `/usr/local/bin`을 PATH 앞으로 옮긴 뒤, 새 터미널에서 `which -a cys` 로 1순위를 확인하세요.
     - **"PATH에서 cys를 찾지 못했습니다"** — 확인은 정상적으로 끝났는데 그 셸의 PATH에
       `cys`가 없는 경우입니다. 대개 PATH에 `/usr/local/bin`이 들어 있지 않아서입니다(설치 자체가
       실패한 것이 아닙니다). 고치는 방법은 바로 아래 **"이 경고가 거짓일 수 있습니다"** 를 보세요.
     - **"확인 불가"** — 확인 명령(`which -a cys`)을 실행하지 못했거나 비정상 종료·무응답이라
       **무엇이 잡히는지 모르는** 경우입니다. 새 터미널에서 `which -a cys` 를 직접 실행해 확인하세요.
       (이 둘은 서로 다른 상황입니다 — 알림 문구가 어느 쪽인지 알려 줍니다.)
   - 확인에 쓰는 셸은 **로그인 셸**(`$SHELL`, macOS 기본은 zsh)입니다. 즉 "PATH 1순위"라는 판정은
     *그 셸 기준*이며, 다른 셸이나 다른 프로필에서는 결과가 다를 수 있습니다.
   - ⚠ **이 경고가 거짓일 수 있습니다 — 확인은 "비대화형 로그인 셸" 기준입니다.**
     앱은 `$SHELL -lc 'which -a cys'` 로 확인하는데, 그렇게 켠 셸은 여러분이 평소 쓰는 **대화형**
     터미널과 읽는 설정 파일이 다릅니다. 실측(2026-08-25):

     | 셸 | `-lc`(앱이 쓰는 확인 방식)가 읽는 파일 | 안 읽는 파일 |
     | --- | --- | --- |
     | zsh | `~/.zshenv` · `~/.zprofile` · `~/.zlogin` | **`~/.zshrc`** |
     | bash | `~/.bash_profile` | **`~/.bashrc`** |

     그래서 PATH를 `~/.zshrc`에만 넣어 둔 분은 **터미널에서 `cys`가 멀쩡히 동작하는데도** 이 경고를
     봅니다. 그때는 **무시해도 됩니다.** (예전 문서는 이 경우에 `~/.zshrc`를 고치라고 안내했는데,
     그대로 해도 경고가 사라지지 않습니다 — 그 셸이 그 파일을 읽지 않기 때문입니다.)
     정말로 PATH를 고쳐야 한다면 **위 표의 "읽는 파일"** 에 넣으세요:
     ```sh
     # 🧑 [HUMAN] zsh 기준 — 대화형·비대화형 양쪽에서 잡히게 하려면 ~/.zprofile 에 넣습니다
     echo 'export PATH="/usr/local/bin:$PATH"' >> ~/.zprofile
     ```
     (앱이 대화형 셸(`-lic`)로 확인하지 **않는** 것은 의도입니다 — 버튼을 누른 부작용으로 여러분의
     `~/.zshrc`가 실행되면 nvm·conda·oh-my-zsh 같은 것이 백그라운드 프로세스를 띄울 수 있습니다.)
   - **같은 이름의 파일이 이미 있을 때**: `/usr/local/bin/cys`(또는 `cysd`) 자리에 심볼릭 링크가
     아닌 **실제 파일**(다른 도구가 설치한 바이너리)이 있으면, 설치는 그 파일을 **지우지 않고**
     같은 폴더에 `<경로>.cys-backup-<시각>` 으로 옮겨 보관한 뒤 링크를 만듭니다. 백업된 경로는
     결과 알림에 나오며, 되돌리려면 그 파일을 원래 이름으로 다시 옮기면 됩니다.
     (다른 앱을 가리키던 **링크**는 백업 없이 새 링크로 바뀝니다 — 링크 자체에는 내용이 없습니다.)
     그런 파일이 있으면 버튼을 누르기 전에 Control Center에 미리 고지가 뜹니다.
   - 앱이 `/Applications` 밖(예: `~/Downloads`, DMG 안)에서 실행 중이면 버튼은 **거부**합니다.
     Finder로 응용 프로그램 폴더에 옮긴 뒤 다시 시도하세요.
2. **폴백 — 수동 sudo:** GUI를 못 쓰는 환경에서만.
```sh
# 🧑 [HUMAN] 🚧 [BOUNDARY INST-DENY-02] sudo 심링크 — 사람이 직접
sudo ln -sfn /Applications/cys.app/Contents/MacOS/cys  /usr/local/bin/cys
sudo ln -sfn /Applications/cys.app/Contents/MacOS/cysd /usr/local/bin/cysd
```
`/usr/local/bin/cys`·`/usr/local/bin/cysd` 심볼릭이 생기고 새 터미널에서 `cys`가 바로
동작합니다. (앱 업데이트에도 경로 유지 — 심볼릭이라 자동 추종. pane *안*에서는 PATH가 자동
주입되므로 이 단계는 **앱 밖 터미널**에서 `cys`를 칠 때만 필요)

**해제 방법** — 같은 버튼이 상태에 따라 **"셸 cys 해제"** 로 바뀝니다. 누르면 확인 창이 먼저 뜨고,
승인하면 `/usr/local/bin/cys`·`cysd` **심볼릭 링크만** 제거합니다(관리자 승인 1회). 같은 이름의
일반 파일이나 다른 앱을 가리키는 링크는 건드리지 않고 사유와 함께 건너뜁니다. 수동 폴백은
`sudo rm /usr/local/bin/cys /usr/local/bin/cysd` (🧑 [HUMAN] — 지우기 전 `ls -l` 로 심링크인지 확인).

건너뛴 항목이 있거나 지운 뒤에도 남은 파일이 있으면 알림이 **"부분 완료"** 로 뜨고(60초 유지 ·
이후 알람 탭), **어떤 경로를 왜 건너뛰었는지**와 남은 경로의 복구 명령(`sudo rm <경로>`)을 그대로
보여줍니다. 즉 일부만 정리된 상태가 성공 알림으로 덮이지 않습니다.

### 🧑 C. 24/365 상시 가동 (선택 — 헤드리스/무인 운영) [HUMAN]
재부팅 후에도 데몬이 자동으로 살아 있게 launchd에 등록:
```sh
# 🧑 [HUMAN] 상시 가동 등록 (가역)
cys daemon install            # 로그인 시 자동 기동 + 사망 시 자동 재기동(KeepAlive)
cys daemon status             # 등록·가동 상태 확인
cys daemon uninstall          # 해제
```
- 이미 데몬이 떠 있으면 `cys daemon install`은 안전을 위해 거부합니다(flock 충돌 방지).
  기존 세션을 정리하고 launchd로 소유권을 넘기려면 `cys daemon install --takeover`
  **🚧 [BOUNDARY INST-DENY-01] ⚠ 가동 중인 세션이 소멸합니다(비가역) — 워커 자율 실행 금지·정지·오너 보고. 사람은 `cys list`로 먼저 확인.**

## Windows

> 코어(named pipe·ConPTY·DSR)는 검증 완료(docs/WINDOWS_VALIDATION.md). 소비자 Windows 배포본은
> **NSIS 자기완결 설치본** `cys_<버전>_x64-setup.exe` **하나**이며, GUI 앱·데몬(cysd)·CLI(cys)와
> 런타임(PortableGit + Python embeddable)이 모두 그 안에 들어 있습니다.
> 근거: `src-tauri/tauri.windows.conf.json` 의 `"targets": ["nsis"]` · `.github/workflows/release.yml`
> 의 `bundle/nsis/cys_*_x64-setup.exe` 수집.
>
> ⚠ **예전 MSI(WiX)는 폐기됐습니다.** `dist-win/*.wxs` 는 참고용 레거시 잔재로, 어떤 릴리스
> 워크플로에서도 빌드·배포되지 않습니다(`dist-win/README.md`). 그 MSI는 PATH를 등록했지만
> **지금 배포되는 setup.exe 는 등록하지 않습니다** — 아래 A의 경고를 보세요.

### 🧑 A. setup.exe 설치 [HUMAN]
1. `cys_<버전>_x64-setup.exe` 실행. 미서명 빌드라 SmartScreen 경고가 뜨면 "추가 정보 → 실행".
2. **현재 사용자** 설치입니다(`installMode: currentUser`) — 관리자 권한 없이 `%LOCALAPPDATA%\cys`
   에 설치됩니다.
3. `cys list` — 데몬이 없으면 자동 기동됩니다.

> ⚠ **설치기는 PATH를 건드리지 않습니다(시스템·사용자 어느 쪽도).** 앱 pane 안에서는 데몬이
> PATH를 자동 주입하므로 `cys`가 바로 동작하지만, **앱 밖 터미널**(PowerShell·cmd)에서 쓰려면
> `%LOCALAPPDATA%\cys` 를 직접 PATH에 추가하거나 전체 경로로 실행하세요. 자동 PATH 편집을 넣지
> 않은 것은 의도입니다 — Windows PATH 자동 편집은 값 잘림·확장 변수 손상 사고가 알려져 있습니다.
> (USER-MANUAL.md §2.4 와 같은 내용입니다.)

### 🧑 B. ZIP (`.exe` 직다운이 막힐 때) [HUMAN]
`cys_<버전>_x64-setup.zip` 은 **위 setup.exe 한 개를 그대로 담은 포장**입니다(기업 프록시·구형
브라우저 대응 · `scripts/make-win-zip.py`). 압축을 풀어 그 안의 `setup.exe` 를 실행하면 A와
같습니다 — 풀어놓은 폴더를 PATH에 넣는 용도가 아닙니다.

### 🧑 C. 상시 가동 (선택) [HUMAN]
```
cys daemon install     # 로그온 시 자동 기동 (작업 스케줄러 ONLOGON)
cys daemon status
cys daemon uninstall
```
※ Windows는 작업 스케줄러에 사망-시-재기동이 없어, CLI 자동기동이 이를 보완합니다.

## 🤖 빠른 시작 (설치 후 공통) [AGENT — 에이전트 실행 스텝]

> 아래는 설치가 끝난 뒤 **에이전트(워커)가 결정론으로 따르는** 부트 스텝입니다. 각 스텝에
> **Goal / verify**가 붙어 있어, "사람용 한 줄"과 달리 워커가 자율로 실행·검증합니다.
> 위 🚧 설치 경계(INST-DENY-01~04)는 이 스텝에서도 그대로 유효합니다.

**Goal(부트 완료의 단일 정의)**: 4종 의무 노드(worker·CSO·reviewer×2)가 생존하고, 프리플라이트가
GREEN이며, 역할 주소가 등록된 상태.

```sh
# 🤖 [AGENT] Step 1 — 관제 보드 (현재 노드 상태 확인)
cys status                                   # 전 노드 1콜 관제 보드
#   verify: 출력에 데몬 가동·노드 목록이 보이면 OK

# 🤖 [AGENT] Step 2 — 역할 에이전트 기동 (디렉티브 자동 주입)
cys launch-agent --role master --agent claude  # 역할 에이전트 기동 + 디렉티브 자동 주입
#   verify: launch-agent가 ①surface 생성 ②CLI 기동 ③절대지침 stdin 주입 ④레지스트리 등록 완료

# 🤖 [AGENT] Step 3 — 일괄 부트 (worker + reviewers)
cys boot                                     # 설치된 CLI 자동 감지 → worker+reviewers 일괄 기동
#   verify: python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_orchestra.py" check  → 4종 생존 GREEN
#   on_fail: python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_preflight.py" --fix  (처방 내장 진단)
```

`agents.json`(`~/.cys/pack/agents.json`)에서 각 에이전트의 기동 명령을 환경에 맞게 수정하세요
(claude/gemini/codex/grok 어댑터 정의).

## 🧑 제거 [HUMAN — 비가역 단계 포함]

```sh
# 🧑 [HUMAN] 상시 가동 해제 (가역)
cys daemon uninstall                         # 상시 가동 해제(설치했다면)
# 🧑 [HUMAN] macOS: 앱을 지우기 전에 Control Center 헤더의 "셸 cys 해제"를 먼저 누르면 심링크가 정리됩니다(§B).
# 🧑 [HUMAN] macOS: Applications에서 cys.app 삭제 + /usr/local/bin/cys{,d} 심링크 제거
# 🧑 [HUMAN] Windows: 설정 → 앱 → 'cys' 제거(setup.exe가 등록한 언인스톨러 · PATH는 애초에 건드리지 않았으므로 정리할 것이 없습니다)
# 🧑 [HUMAN] 🚧 [BOUNDARY INST-DENY-03] ⚠ 비가역 완전 삭제 — 워커 자율 실행 금지·정지·오너 보고
rm -rf ~/.cys ~/.local/state/cys             # pack·트랜스크립트·상태 완전 삭제(선택)
```

## 환경 변수 (요약)

`CYS_SOCKET` · `CYS_NO_AUTOSTART`(1=CLI 자동기동 끔) · `CYS_PACK_DIR` ·
거버넌스: `CYS_LOAD_THRESHOLD`·`CYS_PROC_THRESHOLD`·`CYS_DUP_THRESHOLD`·`CYS_AUTOKILL_DUP`·`CYS_IDLE_SECONDS` ·
자비스: `CYS_TYPING_GUARD_SECS`·`CYS_FEED_REMIND_SECS`·`CYS_MASTER_DEADMAN_SECS`·`CYS_AGENT_AUTORESTART`·`CYS_RECALL_RETAIN_DAYS`·`CYS_TODO_DIRS`
(상세는 USER-MANUAL.md §16)

### v0.14.22 신설 노브 (기본값 = 현행 거동 유지 · 전부 opt-in)

| 변수 | 기본 | 뜻 |
|---|---|---|
| `CYS_QUEUE_MAX_WAIT_SECS` | 0 (=비활성) | 큐 머리 최대 대기(초) — 이 값 이상 기다린 머리는 quiet 임계를 낮춘 '제한 배달(overdue)' 자격을 얻는다. **0=현행 quiet 3s 규칙 그대로**(활성 권장 120). human_typing·pause·빈 좌석 게이트는 어떤 단계에서도 면제되지 않는다 |
| `CYS_QUEUE_OVERDUE_QUIET_SECS` | 1 | overdue(제한 배달) 단계의 quiet 임계(초) — '출력 중 주입 금지'의 하한이라 1 미만 설정은 1로 승격된다 |
| `CYS_QUEUE_STARVE_ALERT_SECS` | 0 (=비활성) | 기아 경보 임계(초) — 큐 머리가 이 값 이상 배달이 막혀 있으면 `queue.starved` 발행(쿨다운 5분 · `queue.depth_high`와 별도 축). **발행뿐, 자동 조치 없음**(활성 권장 600) |
| `CYS_ROLE_DEADMAN_CONFIRM_TICKS` | 3 (최소 1) | 역할(role) 데드맨 v2 — 사망 후보 연속 관측 확증 틱 수 |
| `CYS_ROLE_DEADMAN_GRACE_SECS` | 60 | 역할 데드맨 v2 — 부트/승계 직후 무카운트 창(오살 방지) |
| `CYS_ROLE_DEADMAN_DEBOUNCE_SECS` | 300 | 역할 데드맨 v2 — `master.deadman`(사망 축) 디바운스 |
| `CYS_ROLE_DEADMAN_IDLE_DEBOUNCE_SECS` | =DEBOUNCE(300) | `master.idle`(생존 확정+침묵 정보 신호) 전용 디바운스 — 미설정 시 `CYS_ROLE_DEADMAN_DEBOUNCE_SECS`를 따른다 |
| `CYS_ROLE_DEADMAN_ROLES` | `master` | 역할 데드맨 감시 대상 role CSV(일반화 opt-in). 빈 값은 기본 `master`로 되돌아간다(사망 감시 전면 off 노브는 일부러 없음) |

**★기존 노브 1건의 의미가 바뀌었습니다 — `CYS_MASTER_DEADMAN_SECS`** (이번 판의 유일한
비-additive 변경). `0`은 v0.14.21까지 "무반응 감지 **전체** 끄기"였으나, 이제는 **침묵(idle)
감지만** 끕니다 — 사망 축(`master.deadman`)은 `0`에서도 계속 발화합니다. v1의 침묵-오경보를
끄려고 `0`을 설정해 둔 설치는 업그레이드 후 사망 축 경보를 다시 받게 됩니다(오경보가 아니라
구조 증거 기반). 발화 간격은 `CYS_ROLE_DEADMAN_DEBOUNCE_SECS`로 조정하십시오.
