---
name: timesfm-forecasting
description: >
  Zero-shot 시계열 예측 — Google TimesFM 2.5(200M) foundation model로 학습 없이
  단변량 시계열(매출·수요·센서·바이탈·가격·에너지·기상·과학측정 등)의 점예측 + 분위수
  예측구간을 산출한다. 모델 로드 전 반드시 결정론 preflight(check_timesfm.py)로 RAM·디스크·
  Python·아키텍처를 검증하고, 데이터셋 메모리 fit을 추정한다. cysjavis 규약: 수치는 스크립트
  출력(결정론)이며 해석·문장화만 LLM이 담당한다. 분위수는 '시계열 모델 분위수'이지 이벤트
  확률이 아니며(두 불확실성 합산 금지), 모든 정확도 주장은 hold-out 실측 전까지 '미검증'이다.
license: Apache-2.0
metadata:
  upstream: google-research/timesfm
  upstream_skill_author: Clayton Young (@borealBytes)
  model_repo: google/timesfm-2.5-200m-pytorch
  attribution: >
    TimesFM is licensed Apache-2.0. Not an officially supported Google product —
    self-host 운영 책임은 사용자에게 있다.
---

# TimesFM Forecasting (cysjavis)

## 개요

TimesFM(Time Series Foundation Model)은 Google Research의 decoder-only 사전학습 모델이다.
**zero-shot** — 단변량 시계열을 넣으면 학습 없이 점예측(point)과 분위수 예측구간(quantile)을
돌려준다. 본 스킬은 TimesFM 2.5(200M, torch 경로)를 기본으로 한다.

> **핵심 수치**: TimesFM 2.5 = 200M 파라미터(~800MB 디스크, CPU 약 1.5GB RAM).
> 구버전 2.0(500M)은 ~16GB RAM 필요. 항상 preflight를 먼저 돌려라.

### cysjavis 결정론/LLM 분리 (절대)

- **수치 = 스크립트 출력**: `timesfm_forecast.py`가 내는 JSON의 `point_forecast`·
  `quantile_forecast`가 사실이다. LLM은 이 수치를 **재계산·추론하지 않는다**.
- **해석 = LLM**: 산출된 수치를 사람이 읽는 문장으로 풀이하는 것만 LLM이 한다.
- **게이트 = exit code**: `check_timesfm.py`의 exit code(0/2/3)가 진행 가부의 사실이다.

## 언제 쓰나

- 단변량 시계열의 **zero-shot 예측** (학습 불필요)
- **분위수 예측구간**(q10–q90)이 필요한 확률적 예측
- 임의 길이 시계열(2.5는 최대 16,384 context)
- 수백~수천 series **배치 예측**
- ARIMA/ETS 수동 튜닝 대신 foundation model 접근

## 언제 안 쓰나 (deny)

- 계수 해석이 필요한 고전 통계모형 → `statsmodels`
- 시계열 분류·군집 → `aeon`
- 다변량 VAR·Granger 인과 → `statsmodels`
- 비시계열 표형 데이터 → `scikit-learn`
- **이벤트 발생확률**(예: "다음 분기 침체 확률")을 원할 때 → 본 스킬의 분위수는
  이벤트확률이 아니다. 분위수를 확률로 둔갑시키지 말 것.
- 비수치 indicator(범주 라벨만) → forecasting 입력 부적합 (deny)

## ⚠️ 필수 preflight — 모델 로드 전 반드시 실행

```bash
python3 scripts/check_timesfm.py
```

stdlib만 사용(psutil 등 비표준 의존 없음). 검사: RAM(sysctl/proc/os) · Disk(shutil) ·
Python>=3.10 · Arch(arm64면 PAX/lingvo deny·torch만 허용) · timesfm/torch 설치 · 데이터셋 fit.

**exit code 계약 (이것이 사실)**:

| exit | 판정 | 의미 |
| ---- | ---- | ---- |
| 0 | READY | 안전하게 로드 가능 |
| 2 | WARN | tight (작은 batch·CPU 한정 등으로 가능하나 빡빡) |
| 3 | BLOCK | 요구사항 미달 — 로드 금지 |

데이터셋 메모리 fit 추정:

```bash
python3 scripts/check_timesfm.py --num-series 1000 --context-length 1024 --horizon 24
# 메모리 추정만 (시스템 검사 생략)
python3 scripts/check_timesfm.py --num-series 5000 --context-length 2048 --estimate-only
```

추정 공식: `RAM ≈ 0.8GB(model) + 0.5GB(overhead) + 0.2MB × num_series × context / 1000 (+output) × 1.2`

### 모델 카탈로그

| ver | params | max_context | 비고 |
| --- | ------ | ----------- | ---- |
| 1.0 | 200M | 2048 | archived. 월간데이터 `freq=[0]` 필요 |
| 2.0 | 500M | 2048 | archived. ~16GB RAM |
| **2.5** | **200M** | **16384** | **권장**. freq 플래그 없음 |

> Apple Silicon(arm64): JAX 구경로(PAX/lingvo)는 deny, torch 경로만 허용. MPS latency·
> 메모리는 미실측이므로 wall-clock은 보수적으로 추정한다(아래 ×2 비용 주의).

## forecast 실행

```bash
python3 scripts/timesfm_forecast.py --input data.csv --value-col sales --horizon 24 --output fc.json
# JSON 입력 / 다중 series
python3 scripts/timesfm_forecast.py --input series.json --horizon 12
```

`timesfm`는 lazy import — 미설치 환경에서도 import 단계에서 죽지 않고 안내 후 exit 3.

### ForecastConfig 5 플래그 (P11)

| 플래그 | 기본 | 의미 |
| ------ | ---- | ---- |
| `--normalize-inputs` / `--no-normalize-inputs` | **on** | 입력 정규화(scale 불안정 방지) |
| `--continuous-quantile` / `--no-continuous-quantile` | **on** | continuous quantile head(분위수 collapse 방지) |
| `--fix-quantile-crossing` / `--no-fix-quantile-crossing` | **on** | 분위수 단조성 q10≤…≤q90 |
| `--infer-is-positive` / `--allow-negative` | **infer on** | 입력 모두 >0이면 출력 ≥0 보장. 기온·수익률 등 음수 series는 `--allow-negative` |
| `--flip-invariance` | **off** | `force_flip_invariance`. **True = decode 2회 추론 → 2배 비용**. 음수입력 대칭이 꼭 필요할 때만 |

> **×2 비용 거버넌스**: `--flip-invariance`를 켜면 decode가 2회 돌아 추론 비용이 2배다
> (timesfm_2p5_torch decode 경로). 기본 off로 두어 불필요한 2배 비용을 회피한다.
> watchdog/load 산정 시 켜는 경우 ×2를 보수적 상한으로 등록하라.

## 출력 스키마

`timesfm_forecast.py`가 내는 JSON:

```json
{
  "model_version": "2.5",
  "config": { ... },
  "horizon": 24,
  "point_forecast":    "(N, H)  — median(=q50) 점예측",
  "quantile_forecast": "(N, H, 10) — index0=mean, 1=q10, ..., 9=q90",
  "quantile_index_legend": {"0":"mean","1":"q10","5":"q50_median","9":"q90"},
  "uncertainty_kind": "timeseries_model_quantile(NOT event probability)",
  "caveat": "..."
}
```

분위수 인덱스(off-by-one 주의):

| index | 의미 |
| ----- | ---- |
| **0** | **mean** (q0 아님!) |
| 1 | q10 (80% PI 하한) |
| 5 | q50 = median = `point_forecast` |
| 9 | q90 (80% PI 상한) |

```python
IDX_Q10, IDX_Q90 = 1, 9   # 80% PI
# q[..., 0] 은 mean. q[..., 1] 이 q10.
```

## ⛔ 두 불확실성 합산 금지 (경고)

`quantile_forecast`는 **시계열 모델의 분위수**다. 이것은 **이벤트(사건) 발생확률이 아니다.**

- 시계열 분위수 불확실성(이 스킬) ↔ 이벤트확률 불확실성(예: cross-impact, wild-card)은
  **서로 다른 종류**다.
- 두 종류를 **곱하거나 더하지 말 것**. 함께 제시(병기)만 하라.
- 분위수를 "○○가 일어날 확률 X%"로 번역하지 말 것 — 범주 오류다.

## Quality Checklist

매 TimesFM 작업 후 성공 선언 전 점검:

- [ ] **preflight 통과** — `check_timesfm.py` exit 0(또는 의식적 WARN 수용). BLOCK이면 중단.
- [ ] **출력 shape** — point는 `(N, H)`, quantile은 `(N, H, 10)`.
- [ ] **분위수 인덱스** — index 0=mean, 1=q10 … 9=q90. **0=q0 아님.**
- [ ] **series 길이** — context ≥ 32 포인트 권장.
- [ ] **NaN 없음** — point에 NaN 없어야.
- [ ] **`--infer-is-positive`** — 기온·수익률·음수 가능 series는 `--allow-negative`.
- [ ] **`--flip-invariance`** — 기본 off 유지(2배 비용). 켰다면 watchdog ×2 등록.
- [ ] **불확실성 라벨** — `uncertainty_kind` 보존, 이벤트확률과 합산 안 함.
- [ ] **정확도 라벨** — MAE/coverage 등은 hold-out 실측 전까지 '미검증'.

## Common Mistakes

1. **분위수 off-by-one** — `q[..., 0]`은 **mean**, q0 아님. q10=index 1, q90=index 9.
   `IDX_Q10, IDX_Q90 = 1, 9` 로 못박아라.
2. **분위수를 이벤트확률로 둔갑** — 시계열 분위수는 사건 발생확률이 아니다. 합산·곱 금지.
3. **freq 플래그 혼동** — TimesFM 1.0/2.0은 월간데이터에 `freq=[0]` 필요. **2.5는 freq 없음.**
4. **음수 series에 infer_is_positive on** — 기온·수익률을 0으로 clamp해 왜곡. `--allow-negative`.
5. **flip-invariance 무심코 on** — decode 2회=추론 2배 비용. 음수 대칭 필요 시만.
6. **preflight 생략** — RAM/디스크 미달로 스왑·행. exit code를 사실로 받아라.

## Validation (TimesFM 미설치 환경에서도 가능)

```bash
# 1. 문법(컴파일) — 의존 없이 항상 통과해야
python3 -c "import ast; ast.parse(open('scripts/check_timesfm.py').read()); print('check_timesfm: OK')"
python3 -c "import ast; ast.parse(open('scripts/timesfm_forecast.py').read()); print('timesfm_forecast: OK')"

# 2. preflight 결정론 동작 (모델 로드 없음)
python3 scripts/check_timesfm.py; echo "exit=$?"

# 3. 데이터셋 fit 추정 (시스템 검사 생략)
python3 scripts/check_timesfm.py --num-series 1000 --context-length 1024 --estimate-only; echo "exit=$?"

# 4. forecast lazy import 게이트 — timesfm 미설치 시 BLOCK(exit 3) + 안내
python3 scripts/timesfm_forecast.py --input /dev/null --horizon 4; echo "exit=$?"  # 미설치면 3
```

## 운영 경계 (cysjavis)

- 가중치(~800MB)는 HuggingFace → `~/.cache/huggingface`(또는 `HF_HOME`) 캐시. PACK 텍스트에
  바이너리 봉인 금지(Apache-2.0 귀속 + 'not officially supported Google product' 면책).
- `timesfm_forecast.py`는 장시간 추론 시 `cys run`으로 생명주기 강제종료 권장.
- 모든 정확도(MAE/RMSE/MAPE/coverage) 주장은 **hold-out 실측 전까지 '미검증' 라벨** —
  자가보고 우위 신뢰 금지.

## 출처

- Paper: A Decoder-Only Foundation Model for Time-Series Forecasting (ICML 2024, arXiv:2310.10688)
- HuggingFace: google/timesfm-2.5-200m-pytorch
- upstream: google-research/timesfm (Apache-2.0)
