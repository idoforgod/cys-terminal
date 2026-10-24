# Trigger census at f0ac3b0c

Command: git ls-tree --name-only f0ac3b0c:.github/workflows; each raw/<name>.command.json holds git show command. Here raw files are local-checks/triggers/. No remote results read.

| Workflow | Branch push triggers? |
|---|---|
| ci-branch.yml | NO |
| pack-release.yml | NO |
| release-postprocess.yml | NO |
| release-publish.yml | NO |
| release.yml | NO |
| windows-build.yml | NO |
| windows-health.yml | NO |
| win-measure.yml (new) | YES, only exact branch plus diag/workflow path filter |

## ci-branch.yml

```yaml
on:
  push:
    # ★fix/** 편입(2026-09-04 · 부트 v2 D4 master 판정). 종전엔 feat/** 뿐이라 수리 레인
    #   (fix/0.14.30-*)의 커밋이 **어느 CI 레인에서도 돌지 않았다** — 이 레인 자신이 잡으라고
    #   만들어진 '브랜치 초록의 조기 경보'가 정작 수리 브랜치에 무발화였다는 뜻이다.
    #   릴리스 직전 수리가 태그 레인(20분)에서야 검증되는 것을 막는다. 태그·발행 트리거는
    #   여전히 없다(이 파일은 push 전용 경량 CI).
    branches: ['feat/**', 'fix/**']
  workflow_dispatch: {}
```

## pack-release.yml

```yaml
on:
  push:
    tags:
      - "pack-v*"
```

## release-postprocess.yml

```yaml
on:
  push:
    branches: ['chore/postprocess-*']
  workflow_dispatch:
    inputs:
      tag:
        description: 'Release tag (vX.Y.Z)'
        required: true
        type: string
```

## release-publish.yml

```yaml
on:
  workflow_dispatch:
    inputs:
      tag:
        description: Existing reviewed draft tag (vX.Y.Z)
        required: true
        type: string
      release_bundle_sha256:
        description: SHA-256 of the reviewed SHA256SUMS.txt
        required: true
        type: string
      confirm:
        description: Type PUBLISH to request public promotion
        required: true
        type: string
```

## release.yml

```yaml
on:
  push:
    tags:
      - 'v*'
  workflow_dispatch:
```

## windows-build.yml

```yaml
on:
  push:
    # ★fix/** 편입(2026-09-04 · master 판정 — ci-branch.yml 의 D4 편입과 같은 논리).
    #   Windows 는 설치·봉인·무중단 재설치의 **유일한 실기 검증처**인데, 트리거가
    #   feat/windows-x64-dist 하나뿐이라 수리 레인(fix/0.14.30-*)의 Windows 변경이
    #   **어느 레인에서도 돌지 않았다** — 태그 레인에 가서야 드러난다는 뜻이고, 그때는
    #   되돌리는 비용이 가장 비싸다. IG-4 가 요구하는 'Windows CI green 증적'도 이 트리거
    #   없이는 workflow_dispatch 수단이 있어야만 얻을 수 있었다(실측: gh 미설치 환경에서 불가).
    #   대가는 fix/** 푸시마다 ~20분 레인이 도는 것이며, master 가 품질 우선으로 수용했다.
    #   태그·발행 트리거는 여전히 없다 — 이 파일은 어떤 릴리스도 만들지 않는다.
    branches: ['feat/windows-x64-dist', 'fix/**']
  workflow_dispatch:
```

## windows-health.yml

```yaml
on:
  push:
    branches: ['feat/**', 'fix/**']
    tags: ['v*']
  workflow_dispatch: {}
```

## win-measure.yml (new)

```yaml
on:
  push:
    branches: ['work/0.14.53-win-measure']
    paths: ['diag/**', '.github/workflows/win-measure.yml']
```
