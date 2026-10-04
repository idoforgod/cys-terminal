// Control Center 하드웨어 모니터링(control.hw) — CPU 코어별·GPU·NPU·메모리 스냅샷.
// CPU·MEM은 sysinfo(전 플랫폼). GPU·NPU는 macOS Apple Silicon 전용 —
// GPU는 IOAccelerator PerformanceStatistics(ioreg), NPU 코어수는 칩명 판정.
// 미지원 플랫폼·측정 불가 항목은 null 반환(UI가 "—" 표기).

use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};

// 지속 System — cpu_usage는 직전 refresh와의 델타로 측정된다. 콜마다 System::new+200ms
// 블로킹 sleep을 쓰던 구 패턴은 tokio 워커를 상시 점유했다(전수조사 A-5/B-14 교정).
// 폴링(2~5초) 간격 자체가 측정 창이 되므로 sleep이 불필요하다(부트 후 첫 콜만 0%).
static SYS: OnceLock<Mutex<sysinfo::System>> = OnceLock::new();

fn sys() -> &'static Mutex<sysinfo::System> {
    SYS.get_or_init(|| Mutex::new(sysinfo::System::new()))
}

/// 전체 CPU%·메모리 (control.dashboard 시스템 표시용 — snapshot과 같은 지속 System 공유)
pub fn cpu_mem() -> (f32, u64, u64) {
    let mut s = sys().lock().unwrap();
    s.refresh_memory();
    s.refresh_cpu_usage();
    (s.global_cpu_usage(), s.used_memory(), s.total_memory())
}

pub fn snapshot() -> Value {
    let (per_core, brand, total_pct, mem_used, mem_total) = {
        let mut s = sys().lock().unwrap();
        s.refresh_memory();
        s.refresh_cpu_usage();
        (
            s.cpus().iter().map(|c| c.cpu_usage()).collect::<Vec<f32>>(),
            s.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
            s.global_cpu_usage(),
            s.used_memory(),
            s.total_memory(),
        )
    };
    let (perf, eff) = perf_eff_cores();
    json!({
        "cpu": {
            "cores": per_core.len(),
            "perf_cores": perf,
            "eff_cores": eff,
            "brand": brand,
            "total_pct": total_pct,
            "per_core_pct": per_core,
        },
        "mem": { "total": mem_total, "used": mem_used },
        "gpu": { "cores": gpu_cores(), "pct": gpu_pct() },
        // NPU 활용률(%)은 macOS 공개 API 부재(powermetrics=sudo 전용) — 무권한 실측 가능한
        // 유일 신호인 ANE 전력(W)을 제공하고 pct는 null 고정(환각 지표 생성 금지).
        // status/reason 은 0.14.43 가산 키 — watts 가 null 인 까닭(첫 호출 · 쓸 수 없음)을 구분한다.
        "npu": npu_json(&brand),
    })
}

// "npu" 객체 — cores·pct·watts 는 종전 그대로, status·reason 은 0.14.43 가산 키.
// ★읽기 순서: watts 를 **먼저** 계산한 뒤 status 를 읽는다 — 샘플러(OnceLock)는 첫 power_watts() 호출이
// 초기화하므로, 같은 초기화 결과가 watts(첫 호출은 null)와 status 에 함께 반영된다.
fn npu_json(brand: &str) -> Value {
    let watts = npu_watts();
    npu_obj(npu_cores(brand), watts, npu_status())
}

// status 는 항상, reason 은 쓸 수 없을 때만("ok" 에는 reason 키가 없다).
fn npu_obj(cores: Option<u32>, watts: Option<f64>, (status, reason): (&'static str, Option<&'static str>)) -> Value {
    let mut o = json!({ "cores": cores, "pct": null, "watts": watts, "status": status });
    if let Some(r) = reason {
        o["reason"] = json!(r);
    }
    o
}

// NPU 가용 상태 판정(순수 함수) — 샘플러 초기화 결과 → (npu.status, npu.reason).
//   Ok = 초기화됨("ok" — 첫 호출이라 watts 가 null 이어도 ok) · Err(사유) = ("unavailable", 사유).
//   사유: "dylib"(dlopen 실패) · "symbol"(심볼 누락) · "no_channel"(ANE 에너지 채널 없음 — 인텔 맥·일부 VM) ·
//   "subscribe"(채널 사본·구독 생성 실패) · "platform"(macOS 아님). 제네릭인 까닭: 검체가 CF 포인터를 쥔 실제
//   샘플러 없이도 판정만 핀할 수 있다.
fn npu_status_of<T>(init: &Result<T, &'static str>) -> (&'static str, Option<&'static str>) {
    match init {
        Ok(_) => ("ok", None),
        Err(reason) => ("unavailable", Some(*reason)),
    }
}

// ─── macOS (Apple Silicon) ───

#[cfg(target_os = "macos")]
fn sysctl_u32(name: &str) -> Option<u32> {
    let out = std::process::Command::new("sysctl").args(["-n", name]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

// P/E 코어 분해 — Apple Silicon 전용 sysctl(Intel 맥은 미존재 → None)
#[cfg(target_os = "macos")]
fn perf_eff_cores() -> (Option<u32>, Option<u32>) {
    (sysctl_u32("hw.perflevel0.logicalcpu"), sysctl_u32("hw.perflevel1.logicalcpu"))
}

// GPU 코어 수 — AGXAccelerator(Apple GPU)의 gpu-core-count. 불변이라 1회 조회 후 캐시.
#[cfg(target_os = "macos")]
fn gpu_cores() -> Option<u32> {
    static CACHE: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *CACHE.get_or_init(gpu_cores_probe)
}

#[cfg(target_os = "macos")]
fn gpu_cores_probe() -> Option<u32> {
    let out = std::process::Command::new("ioreg")
        .args(["-rc", "AGXAccelerator", "-d", "1"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().find(|l| l.contains("\"gpu-core-count\""))?;
    line.rsplit('=').next()?.trim().parse().ok()
}

// GPU 활용률 — IOAccelerator PerformanceStatistics의 "Device Utilization %"(무권한 실측)
#[cfg(target_os = "macos")]
fn gpu_pct() -> Option<f64> {
    let out = std::process::Command::new("ioreg")
        .args(["-r", "-d", "1", "-w0", "-c", "IOAccelerator"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let key = "\"Device Utilization %\"=";
    let i = s.find(key)? + key.len();
    let digits: String = s[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

// Neural Engine 코어 수 — OS 미노출이라 칩명 판정(M시리즈 전 세대 16, Ultra만 2다이=32)
#[cfg(target_os = "macos")]
fn npu_cores(brand: &str) -> Option<u32> {
    if !brand.starts_with("Apple M") {
        return None;
    }
    Some(if brand.contains("Ultra") { 32 } else { 16 })
}

// NPU(ANE) 전력 — 연속 호출 간 IOReport 에너지 델타(J)/경과시간(s)=W. 첫 호출은 None.
#[cfg(target_os = "macos")]
fn npu_watts() -> Option<f64> {
    ane::power_watts()
}

// NPU 가용 상태 — 샘플러 초기화 결과를 (status, reason) 으로. snapshot 이 npu_watts() 뒤에 읽는다.
#[cfg(target_os = "macos")]
fn npu_status() -> (&'static str, Option<&'static str>) {
    ane::status()
}

// ─── ANE 전력 실측 — IOReport private dylib (macmon(MIT) 검증 기법의 클린룸 최소 포트) ───
// "Energy Model" 그룹의 ANE* 채널(Basic="ANE"·Max="ANE0"·Ultra="ANE0_{n}") 누적 에너지를
// 구독해 두 스냅샷 델타로 전력을 산출한다. 채널이 없거나(인텔 맥·일부 VM) 호출이 실패하면
// None(UI "—") — 관측 전용 fail-open이며 데몬 동작에 영향을 주지 않는다.
//
// ★강링크 금지(0.14.43) — 이 모듈은 한때 비공개 libIOReport.dylib 을 link 속성(kind=dylib)으로 프로세스에
//   강하게 묶었다. 강링크는 **로드 시점 의존**이다: dyld 가 프로세스 시작 때 풀기 때문에, 앞으로의 macOS 에서
//   그 dylib 이 없어지거나 8개 심볼 중 하나라도 빠지면 cysd 가 main 에 닿기도 전에 "Library not loaded"/
//   "Symbol not found" 로 죽는다(데몬 기동 불능 = 모든 pane 사망). NPU 전력은 Control Center 의 관측용 숫자
//   하나일 뿐이라 그 숫자가 데몬 기동을 담보로 잡을 수 없다. 그래서 8개 함수를 첫 호출 때 dlopen/dlsym 으로
//   찾고(지연 로딩), 못 찾으면 NPU 전력만 꺼진다(watts:null · npu.status="unavailable").
//   · 경로: 최근 macOS 는 시스템 dylib 을 dyld 공유 캐시에서 해석한다. /usr/lib/libIOReport.dylib 이 디스크에
//     파일로 안 보여도 dlopen 은 성공하므로, 경로 존재를 따로 검사하지 않고 dlopen 결과로만 판정한다.
//   · 부분 심볼 금지: 8개 중 하나라도 없으면 통째로 '사용 불가'다. 일부만 쥔 채 진행하면 null 함수 포인터
//     호출(SIGSEGV = 데몬 사망)이거나, ANE 판정·델타·단위 환산 중 한 단계가 조용히 빠져 거짓 전력값이 나온다.
//   · CoreFoundation 은 공개 프레임워크라 아래처럼 그대로 강링크한다(바뀐 건 IOReport 뿐).
#[cfg(target_os = "macos")]
mod ane {
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};
    use std::ptr::null;
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    type CFTypeRef = *const c_void;
    type CFDictionaryRef = CFTypeRef;
    type CFMutableDictionaryRef = CFTypeRef;
    type CFStringRef = CFTypeRef;
    type CFArrayRef = CFTypeRef;
    type CFIndex = isize;
    const K_UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFDictionaryGetValue(d: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
        fn CFDictionaryCreateMutableCopy(alloc: CFTypeRef, cap: CFIndex, d: CFDictionaryRef) -> CFMutableDictionaryRef;
        fn CFArrayGetCount(a: CFArrayRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(a: CFArrayRef, i: CFIndex) -> CFTypeRef;
        fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFStringRef;
        fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: CFIndex, enc: u32) -> bool;
    }

    // IOReport 8개 함수의 시그니처 — 종전 extern 선언의 매개변수·반환형과 글자 그대로 같다(이름만 별칭).
    type CopyAllChannelsFn = unsafe extern "C" fn(u64, u64) -> CFDictionaryRef;
    type CreateSubscriptionFn = unsafe extern "C" fn(*const c_void, CFMutableDictionaryRef, *mut CFMutableDictionaryRef, u64, CFTypeRef) -> CFTypeRef;
    type CreateSamplesFn = unsafe extern "C" fn(CFTypeRef, CFMutableDictionaryRef, CFTypeRef) -> CFDictionaryRef;
    type CreateSamplesDeltaFn = unsafe extern "C" fn(CFDictionaryRef, CFDictionaryRef, CFTypeRef) -> CFDictionaryRef;
    type ChannelGetGroupFn = unsafe extern "C" fn(CFDictionaryRef) -> CFStringRef;
    type ChannelGetChannelNameFn = unsafe extern "C" fn(CFDictionaryRef) -> CFStringRef;
    type ChannelGetUnitLabelFn = unsafe extern "C" fn(CFDictionaryRef) -> CFStringRef;
    type SimpleGetIntegerValueFn = unsafe extern "C" fn(CFDictionaryRef, i32) -> i64;

    /// IOReport 8개 함수 표 — dlopen/dlsym 으로 런타임에 채운다(위 ★강링크 금지).
    /// **전부 있거나 전혀 없다**: [`load_api_from`] 은 한 심볼이라도 못 찾으면 표를 만들지 않는다.
    #[derive(Clone, Copy)]
    pub(super) struct IoReportApi {
        copy_all_channels: CopyAllChannelsFn,              // IOReportCopyAllChannels
        create_subscription: CreateSubscriptionFn,         // IOReportCreateSubscription
        create_samples: CreateSamplesFn,                   // IOReportCreateSamples
        create_samples_delta: CreateSamplesDeltaFn,        // IOReportCreateSamplesDelta
        channel_get_group: ChannelGetGroupFn,              // IOReportChannelGetGroup
        channel_get_channel_name: ChannelGetChannelNameFn, // IOReportChannelGetChannelName
        channel_get_unit_label: ChannelGetUnitLabelFn,     // IOReportChannelGetUnitLabel
        simple_get_integer_value: SimpleGetIntegerValueFn, // IOReportSimpleGetIntegerValue
    }

    /// IOReport 를 쓸 수 없는 이유 — `control.hw` 의 `npu.reason`("dylib"·"symbol")으로 노출된다.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum ApiErr {
        /// dlopen 실패 — dylib 을 열 수 없다(OS 가 없앴거나 경로가 바뀜).
        Dylib,
        /// dylib 은 열렸으나 8개 심볼 중 하나가 없다(값 = 처음 못 찾은 심볼 이름).
        Symbol(&'static str),
    }

    impl ApiErr {
        /// `npu.reason` 문자열(공개 계약).
        fn reason(self) -> &'static str {
            match self {
                ApiErr::Dylib => "dylib",
                ApiErr::Symbol(_) => "symbol",
            }
        }
    }

    /// 기본 경로. 공유 캐시에서 해석되므로 디스크에 파일이 없어도 dlopen 이 성공한다(위 머리말).
    pub(super) const IOREPORT_DYLIB: &CStr = c"/usr/lib/libIOReport.dylib";
    /// 기본 경로가 실패하면 leaf 이름으로 한 번 더 — dyld 의 기본 탐색 경로가 해석한다.
    pub(super) const IOREPORT_LEAF: &CStr = c"libIOReport.dylib";
    /// 시도 순서 — 기본 경로 → leaf 이름(기본 경로가 실패했을 때만 한 번 더).
    pub(super) const DEFAULT_PATHS: [&CStr; 2] = [IOREPORT_DYLIB, IOREPORT_LEAF];

    /// dlsym 한 개 — null(없음)이면 `Err(Symbol(이름))`. `Ok` 의 포인터는 항상 null 이 아니다.
    unsafe fn find(handle: *mut c_void, name: &'static CStr) -> Result<*mut c_void, ApiErr> {
        let p = libc::dlsym(handle, name.as_ptr());
        if p.is_null() {
            return Err(ApiErr::Symbol(name.to_str().unwrap_or("?")));
        }
        Ok(p)
    }

    /// `path` 의 dylib 을 열어 IOReport 8개 심볼을 **전부** 찾는다. 열리지 않으면 `Err(Dylib)` · 심볼이 하나라도
    /// 없으면 `Err(Symbol(처음 없는 이름))` — 부분 표는 돌려주지 않는다.
    ///
    /// `dlclose` 는 부르지 않는다: 표의 함수 포인터가 프로세스 수명 동안 쓰이므로(`API` static) 핸들도 끝까지 연다.
    /// 실패 경로에서도 닫지 않는다(시스템 dylib 은 공유 캐시 상주라 참조 하나를 더 쥘 뿐이고, 시도는 프로세스당
    /// 최대 2회다).
    ///
    /// # Safety
    /// 돌려준 함수 포인터는 그 dylib 의 심볼이 종전 extern 선언과 같은 시그니처일 때만 유효하다.
    pub(super) unsafe fn load_api_from(path: &CStr) -> Result<IoReportApi, ApiErr> {
        let h = libc::dlopen(path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if h.is_null() {
            return Err(ApiErr::Dylib);
        }
        // 필드 순서 = 평가 순서 — 처음 없는 심볼이 보고된다. transmute 는 find 의 null 검사를 통과한 **뒤**에만 일어난다.
        Ok(IoReportApi {
            copy_all_channels: std::mem::transmute::<*mut c_void, CopyAllChannelsFn>(find(h, c"IOReportCopyAllChannels")?),
            create_subscription: std::mem::transmute::<*mut c_void, CreateSubscriptionFn>(find(h, c"IOReportCreateSubscription")?),
            create_samples: std::mem::transmute::<*mut c_void, CreateSamplesFn>(find(h, c"IOReportCreateSamples")?),
            create_samples_delta: std::mem::transmute::<*mut c_void, CreateSamplesDeltaFn>(find(h, c"IOReportCreateSamplesDelta")?),
            channel_get_group: std::mem::transmute::<*mut c_void, ChannelGetGroupFn>(find(h, c"IOReportChannelGetGroup")?),
            channel_get_channel_name: std::mem::transmute::<*mut c_void, ChannelGetChannelNameFn>(find(h, c"IOReportChannelGetChannelName")?),
            channel_get_unit_label: std::mem::transmute::<*mut c_void, ChannelGetUnitLabelFn>(find(h, c"IOReportChannelGetUnitLabel")?),
            simple_get_integer_value: std::mem::transmute::<*mut c_void, SimpleGetIntegerValueFn>(find(h, c"IOReportSimpleGetIntegerValue")?),
        })
    }

    /// 경로를 차례로 시도해 처음 성공한 표를 돌려준다. 전부 실패하면 `Symbol` 이 있으면 그것(dylib 은 열렸으나
    /// 심볼이 빠졌다는 더 구체적인 진단)을, 없으면 `Dylib` 을 돌려준다.
    pub(super) unsafe fn load_api_chain(paths: &[&CStr]) -> Result<IoReportApi, ApiErr> {
        let mut err = ApiErr::Dylib;
        for p in paths {
            match load_api_from(p) {
                Ok(api) => return Ok(api),
                Err(ApiErr::Symbol(name)) if err == ApiErr::Dylib => err = ApiErr::Symbol(name),
                Err(_) => {}
            }
        }
        Err(err)
    }

    static API: OnceLock<Result<IoReportApi, ApiErr>> = OnceLock::new();

    /// IOReport 함수 표 — 프로세스 수명 동안 **1회만** 로드한다(기본 경로 → leaf 이름 순). 못 얻으면 사유를 돌려준다.
    pub(super) fn api() -> Result<&'static IoReportApi, ApiErr> {
        API.get_or_init(|| {
            let r = unsafe { load_api_chain(&DEFAULT_PATHS) };
            // 프로세스당 1회 — 못 찾은 심볼 이름은 RPC(`npu.reason`)에 실리지 않으므로 데몬 로그에 남긴다.
            match &r {
                Err(ApiErr::Dylib) => eprintln!("[cysd] hwmon: libIOReport.dylib 을 열 수 없다 — NPU 전력을 끈다(watts:null)"),
                Err(ApiErr::Symbol(name)) => eprintln!("[cysd] hwmon: IOReport 심볼 {name} 없음 — NPU 전력을 끈다(watts:null)"),
                Ok(_) => {}
            }
            r
        })
        .as_ref()
        .map_err(|e| *e)
    }

    pub(super) struct Sampler {
        api: IoReportApi,
        subs: CFTypeRef,
        chans: CFMutableDictionaryRef,
        prev: Option<(CFDictionaryRef, Instant)>,
    }
    // raw 포인터는 CF 불변 규약(구독·채널 dict는 생성 후 읽기 전용) + Mutex 직렬화로 안전(함수 포인터 표는 불변)
    unsafe impl Send for Sampler {}

    /// 샘플러 초기화 결과(1회) — 실패하면 사유 문자열("dylib"·"symbol"·"no_channel"·"subscribe")을 남긴다.
    /// `control.hw` 의 `npu.status`/`npu.reason` 이 이것을 읽는다.
    static SAMPLER: OnceLock<Result<Mutex<Sampler>, &'static str>> = OnceLock::new();

    fn sampler() -> &'static Result<Mutex<Sampler>, &'static str> {
        SAMPLER.get_or_init(|| unsafe { init() })
    }

    pub fn power_watts() -> Option<f64> {
        let m = sampler().as_ref().ok()?;
        let mut s = m.lock().ok()?;
        let api = s.api;
        unsafe {
            let cur = (api.create_samples)(s.subs, s.chans, null());
            if cur.is_null() {
                return None;
            }
            let out = match s.prev.take() {
                Some((prev, t0)) => {
                    let dt = t0.elapsed().as_secs_f64();
                    let delta = (api.create_samples_delta)(prev, cur, null());
                    CFRelease(prev);
                    let w = if delta.is_null() { None } else { ane_watts(&api, delta, dt) };
                    if !delta.is_null() {
                        CFRelease(delta);
                    }
                    w
                }
                None => None, // 첫 호출 — 기준 스냅샷만 확보
            };
            s.prev = Some((cur, Instant::now()));
            out
        }
    }

    /// `npu.status`/`npu.reason` — 샘플러 초기화 결과(아직이면 지금 초기화한다)를 순수 판정 함수에 넘긴다.
    pub fn status() -> (&'static str, Option<&'static str>) {
        super::npu_status_of(sampler())
    }

    unsafe fn init() -> Result<Mutex<Sampler>, &'static str> {
        init_with(api())
    }

    /// API 표로 샘플러를 만든다. 표가 없으면 IOReport 를 한 번도 부르지 않고 사유만 낸다.
    /// 사유 = `npu.reason`: 표 없음 → "dylib"/"symbol" · 채널 없음(인텔 맥·일부 VM) → "no_channel" ·
    /// 채널 사본·구독 생성 실패 → "subscribe".
    pub(super) unsafe fn init_with(api: Result<&IoReportApi, ApiErr>) -> Result<Mutex<Sampler>, &'static str> {
        let api = *api.map_err(ApiErr::reason)?;
        let all = (api.copy_all_channels)(0, 0);
        if all.is_null() {
            return Err("no_channel");
        }
        // ANE 에너지 채널이 있는 기기만 활성화(인텔 맥·일부 VM 제외)
        let has_ane = channels(all).any(|it| is_ane_energy(&api, it));
        if !has_ane {
            CFRelease(all);
            return Err("no_channel");
        }
        let chans = CFDictionaryCreateMutableCopy(null(), 0, all);
        CFRelease(all);
        if chans.is_null() {
            return Err("subscribe");
        }
        let mut sub_out: CFMutableDictionaryRef = null();
        let subs = (api.create_subscription)(null(), chans, &mut sub_out, 0, null());
        if subs.is_null() {
            CFRelease(chans);
            return Err("subscribe");
        }
        Ok(Mutex::new(Sampler { api, subs, chans, prev: None }))
    }

    // 델타 dict의 ANE 채널 에너지(J) 합산 → /dt = W
    unsafe fn ane_watts(api: &IoReportApi, delta: CFDictionaryRef, dt: f64) -> Option<f64> {
        if dt <= 0.0 {
            return None;
        }
        let mut joules = 0.0;
        let mut seen = false;
        for it in channels(delta) {
            if !is_ane_energy(api, it) {
                continue;
            }
            let scale = match cf_str((api.channel_get_unit_label)(it)).trim() {
                "mJ" => 1e3,
                "uJ" => 1e6,
                "nJ" => 1e9,
                _ => continue,
            };
            joules += (api.simple_get_integer_value)(it, 0) as f64 / scale;
            seen = true;
        }
        if seen {
            Some(joules / dt)
        } else {
            None
        }
    }

    unsafe fn is_ane_energy(api: &IoReportApi, item: CFDictionaryRef) -> bool {
        cf_str((api.channel_get_group)(item)) == "Energy Model"
            && cf_str((api.channel_get_channel_name)(item)).starts_with("ANE")
    }

    unsafe fn channels(dict: CFDictionaryRef) -> impl Iterator<Item = CFDictionaryRef> {
        let ck = std::ffi::CString::new("IOReportChannels").unwrap();
        let key = CFStringCreateWithCString(null(), ck.as_ptr(), K_UTF8);
        let arr: CFArrayRef = if key.is_null() { null() } else { CFDictionaryGetValue(dict, key) };
        if !key.is_null() {
            CFRelease(key);
        }
        let n = if arr.is_null() { 0 } else { CFArrayGetCount(arr) };
        (0..n).map(move |i| CFArrayGetValueAtIndex(arr, i)).filter(|it| !it.is_null())
    }

    unsafe fn cf_str(s: CFStringRef) -> String {
        if s.is_null() {
            return String::new();
        }
        let mut buf = [0 as c_char; 128];
        if CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as CFIndex, K_UTF8) {
            std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
        } else {
            String::new()
        }
    }
}

// ─── 타 플랫폼 스텁 (Windows GPU/NPU 카운터는 후속 트랙) ───

#[cfg(not(target_os = "macos"))]
fn perf_eff_cores() -> (Option<u32>, Option<u32>) {
    (None, None)
}

#[cfg(not(target_os = "macos"))]
fn gpu_cores() -> Option<u32> {
    None
}

#[cfg(not(target_os = "macos"))]
fn gpu_pct() -> Option<f64> {
    None
}

#[cfg(not(target_os = "macos"))]
fn npu_cores(_brand: &str) -> Option<u32> {
    None
}

#[cfg(not(target_os = "macos"))]
fn npu_watts() -> Option<f64> {
    None
}

#[cfg(not(target_os = "macos"))]
fn npu_status() -> (&'static str, Option<&'static str>) {
    npu_status_of::<()>(&Err("platform"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn snapshot_has_all_sections() {
        let v = super::snapshot();
        assert!(v["cpu"]["cores"].as_u64().unwrap() > 0);
        assert_eq!(v["cpu"]["per_core_pct"].as_array().unwrap().len(), v["cpu"]["cores"].as_u64().unwrap() as usize);
        assert!(v["mem"]["total"].as_u64().unwrap() > 0);
        // Apple Silicon 실기에서는 GPU 코어수·활용률이 실측된다
        #[cfg(target_os = "macos")]
        {
            assert!(v["gpu"]["cores"].as_u64().unwrap_or(0) > 0);
            assert!(v["gpu"]["pct"].as_f64().is_some());
            assert!(v["npu"]["cores"].as_u64().unwrap_or(0) > 0);
        }
    }

    // ANE 전력은 연속 호출 델타라 2번째 스냅샷부터 값이 나온다.
    // CI VM은 IOReport ANE 채널이 없을 수 있어 강제 검증은 CYS_HW_STRICT=1(로컬 실기)로만.
    #[cfg(target_os = "macos")]
    #[test]
    fn ane_power_second_sample() {
        let _ = super::snapshot();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let v = super::snapshot();
        if std::env::var("CYS_HW_STRICT").is_ok() {
            let w = v["npu"]["watts"].as_f64();
            assert!(w.is_some(), "Apple Silicon 실기에서 ANE watts 기대: {v}");
            assert!(w.unwrap() >= 0.0);
        }
    }

    // ─── 0.14.43 E2: IOReport 강링크 → 지연 로딩(dlopen) · npu.status/reason ───

    // npu.reason 의 닫힌 집합(티켓 계약) — 이 밖의 값이 나오면 적색.
    const NPU_REASONS: [&str; 5] = ["dylib", "symbol", "no_channel", "subscribe", "platform"];

    // 소스 핀 — IOReport 강링크(link 속성으로 묶기)가 다시 들어오면 적색. 문자열을 쪼개 써서 이 검체 자신의
    // 글자와 겹치지 않는다(겹치면 스스로 적색이 된다).
    #[test]
    fn e2_source_has_no_ioreport_strong_link() {
        let src = include_str!("hwmon.rs");
        // 종전의 정확한 문구(공백 포함)
        let literal = ["#[li", "nk(name = \"IO", "Report\""].concat();
        assert!(!src.contains(&literal), "IOReport 강링크가 소스에 다시 들어왔다: {literal}");
        // 공백·줄바꿈·인자 순서가 달라진 변형도 같은 강링크다 — 모든 link 속성에서 IOReport 를 찾는다.
        let squashed: String = src.split_whitespace().collect();
        let open = ["#[li", "nk("].concat();
        let target = ["IO", "Report"].concat();
        let mut rest = squashed.as_str();
        let mut seen = 0;
        while let Some(i) = rest.find(&open) {
            let tail = &rest[i..];
            let end = tail.find(")]").expect("link 속성이 닫히지 않았다");
            assert!(!tail[..end].contains(&target), "link 속성이 IOReport 를 묶는다: {}", &tail[..end]);
            seen += 1;
            rest = &tail[end..];
        }
        // 계측 유효성 — CoreFoundation 의 link 속성은 남아 있어야 한다(스캐너가 속성을 실제로 읽고 있다는 증거).
        assert!(seen >= 1, "link 속성 스캐너가 아무것도 읽지 못했다");
    }

    // 상태 판정(순수 함수) — Ok = "ok"(reason 없음) · Err(사유) = "unavailable"+사유. 이 표가 status 를 늘 "ok" 로
    // 만드는 돌연변이(M2)를 막는다.
    #[test]
    fn e2_npu_status_of_maps_init_result() {
        assert_eq!(super::npu_status_of::<()>(&Ok(())), ("ok", None));
        for r in NPU_REASONS {
            assert_eq!(super::npu_status_of::<()>(&Err(r)), ("unavailable", Some(r)));
        }
    }

    // npu 객체 조립 — status 는 항상, reason 은 쓸 수 없을 때만. 기존 키(cores·pct·watts)의 값은 그대로.
    #[test]
    fn e2_npu_obj_adds_status_and_reason_only_when_unavailable() {
        let ok = super::npu_obj(Some(16), Some(1.5), ("ok", None));
        assert_eq!(ok, serde_json::json!({"cores": 16, "pct": null, "watts": 1.5, "status": "ok"}));
        let off = super::npu_obj(None, None, ("unavailable", Some("symbol")));
        assert_eq!(off, serde_json::json!({"cores": null, "pct": null, "watts": null, "status": "unavailable", "reason": "symbol"}));
        // 비 macOS 스텁(npu_status = npu_status_of(Err("platform")))이 내는 모양 — 윈도우·리눅스도 같은 가산 키를 낸다.
        let stub = super::npu_obj(None, None, super::npu_status_of::<()>(&Err("platform")));
        assert_eq!(stub, serde_json::json!({"cores": null, "pct": null, "watts": null, "status": "unavailable", "reason": "platform"}));
    }

    // control.hw 의 npu 객체 계약 — status 는 "ok"|"unavailable", unavailable 이면 reason 이 닫힌 집합 중 하나(그리고
    // watts null) · ok 이면 reason 키가 없다. 기존 키(cores·pct·watts)는 그대로 있다. 모든 플랫폼에서 돈다.
    #[test]
    fn e2_snapshot_npu_status_contract() {
        let v = super::snapshot();
        let npu = &v["npu"];
        assert!(npu.get("cores").is_some() && npu["pct"].is_null() && npu.get("watts").is_some(), "기존 키 변형: {npu}");
        match npu["status"].as_str().expect("npu.status 는 문자열") {
            "ok" => assert!(npu.get("reason").is_none(), "ok 에는 reason 키가 없다: {npu}"),
            "unavailable" => {
                let reason = npu["reason"].as_str().expect("unavailable 에는 reason 문자열이 있다");
                assert!(NPU_REASONS.contains(&reason), "알 수 없는 reason: {reason}");
                assert!(npu["watts"].is_null(), "unavailable 이면 watts 는 null: {npu}");
            }
            other => panic!("status 는 ok|unavailable 이어야 한다: {other} / {npu}"),
        }
    }

    // 비 macOS 스텁도 같은 가산 키를 낸다 — 윈도우·리눅스: status:"unavailable", reason:"platform", watts null.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn e2_non_macos_stub_reports_platform() {
        let v = super::snapshot();
        assert_eq!(v["npu"]["status"], "unavailable");
        assert_eq!(v["npu"]["reason"], "platform");
        assert!(v["npu"]["watts"].is_null());
    }

    // dlopen 실패(없는 경로) → Err(Dylib) · 패닉 없음.
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_load_api_from_missing_path_is_dylib_err() {
        let r = unsafe { super::ane::load_api_from(c"/nonexistent/cys-e2/libIOReport.dylib") };
        assert_eq!(r.err(), Some(super::ane::ApiErr::Dylib));
    }

    // dylib 은 열리지만 IOReport 심볼이 없다 → Err(Symbol) — 부분 사용 금지(표가 통째로 '사용 불가').
    // libSystem 은 모든 프로세스가 이미 올린 시스템 dylib 이라 항상 열린다(M1: 심볼 하나가 null 이어도 Ok 를 내면 적색).
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_load_api_from_dylib_without_ioreport_symbols_is_symbol_err() {
        let r = unsafe { super::ane::load_api_from(c"/usr/lib/libSystem.B.dylib") };
        match r {
            Err(super::ane::ApiErr::Symbol(name)) => assert!(name.starts_with("IOReport"), "심볼 이름: {name}"),
            other => panic!("Err(Symbol) 기대 — got {:?}", other.err()),
        }
    }

    // 기본 경로(공유 캐시가 해석) — CYS_HW_STRICT 가 있으면 Ok 를 단언, 없으면 결과를 출력만(CI VM 은 못 열 수 있다).
    // 기존 ane_power_second_sample 의 관례와 같다.
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_load_api_from_default_path() {
        let r = unsafe { super::ane::load_api_from(super::ane::IOREPORT_DYLIB) };
        let shown = match &r {
            Ok(_) => "Ok(8개 심볼 전부)".to_string(),
            Err(e) => format!("Err({e:?})"),
        };
        println!("load_api_from({:?}) -> {shown}", super::ane::IOREPORT_DYLIB);
        if std::env::var("CYS_HW_STRICT").is_ok() {
            assert!(r.is_ok(), "Apple Silicon 실기에서 IOReport dlopen 기대: {shown}");
        }
    }

    // 시도 순서 핀 — 기본 경로 다음에 leaf 이름 폴백이 있어야 한다(티켓 계약). 폴백을 빼는 돌연변이(M4)를 막는다.
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_default_paths_try_absolute_then_leaf() {
        assert_eq!(super::ane::DEFAULT_PATHS, [c"/usr/lib/libIOReport.dylib", c"libIOReport.dylib"]);
    }

    // leaf 이름 폴백 경로 — 기본 경로와 같은 결과여야 한다(둘 다 공유 캐시가 해석). 결과 출력, STRICT 면 Ok.
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_load_api_from_leaf_name() {
        let r = unsafe { super::ane::load_api_from(super::ane::IOREPORT_LEAF) };
        let shown = match &r {
            Ok(_) => "Ok(8개 심볼 전부)".to_string(),
            Err(e) => format!("Err({e:?})"),
        };
        println!("load_api_from({:?}) -> {shown}", super::ane::IOREPORT_LEAF);
        if std::env::var("CYS_HW_STRICT").is_ok() {
            assert!(r.is_ok(), "Apple Silicon 실기에서 leaf 이름 dlopen 기대: {shown}");
        }
    }

    // 경로 사슬 — 첫 경로가 없어도 다음 경로를 시도하고, 전부 실패하면 더 구체적인 사유(Symbol)를 보고한다.
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_load_api_chain_falls_back_and_reports_most_specific_error() {
        use super::ane::{load_api_chain, load_api_from, ApiErr, IOREPORT_DYLIB};
        let a = c"/nonexistent/cys-e2/a.dylib";
        let b = c"/nonexistent/cys-e2/b.dylib";
        let libsys = c"/usr/lib/libSystem.B.dylib";
        unsafe {
            assert_eq!(load_api_chain(&[a, b]).err(), Some(ApiErr::Dylib));
            assert_eq!(load_api_chain(&[]).err(), Some(ApiErr::Dylib));
            // dylib 은 열렸으나 심볼이 빠진 쪽이 더 구체적인 진단이다 — 순서와 무관하게 Symbol.
            assert!(matches!(load_api_chain(&[a, libsys]).err(), Some(ApiErr::Symbol(_))));
            assert!(matches!(load_api_chain(&[libsys, a]).err(), Some(ApiErr::Symbol(_))));
            // 첫 경로가 없어도 둘째가 열리면 표를 얻는다 — 이 기계에서 기본 경로가 열릴 때만 단언한다
            // (열리는지는 위 e2_load_api_from_default_path 가 STRICT 로 따로 핀한다).
            if load_api_from(IOREPORT_DYLIB).is_ok() {
                assert!(load_api_chain(&[a, IOREPORT_DYLIB]).is_ok(), "첫 경로 실패 뒤 다음 경로를 시도하지 않았다");
            }
        }
    }

    // 표는 프로세스당 1회만 로드한다 — 두 번 불러도 같은 표(같은 주소)다.
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_api_is_loaded_once() {
        match (super::ane::api(), super::ane::api()) {
            (Ok(x), Ok(y)) => assert!(std::ptr::eq(x, y), "표가 두 번 로드되었다"),
            (Err(x), Err(y)) => assert_eq!(x, y),
            (x, y) => panic!("같은 프로세스에서 결과가 갈렸다: {:?} / {:?}", x.err(), y.err()),
        }
    }

    // load_api_from(없는 경로) → 샘플러 초기화 → 상태 판정까지 이어 붙인 검체 — "dylib" 이 npu.reason 으로 흐른다.
    // (M2: 상태 판정이 늘 "ok" 면 적색.)
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_missing_dylib_flows_to_unavailable_dylib() {
        use super::ane;
        unsafe {
            let api = ane::load_api_from(c"/nonexistent/cys-e2/libIOReport.dylib");
            let init = ane::init_with(api.as_ref().map_err(|e| *e));
            assert_eq!(super::npu_status_of(&init), ("unavailable", Some("dylib")));
        }
    }

    // 심볼이 빠진 dylib(libSystem) → "symbol" — 표가 없으니 IOReport 함수는 한 번도 불리지 않는다.
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_partial_symbols_flow_to_unavailable_symbol() {
        use super::ane;
        unsafe {
            let api = ane::load_api_from(c"/usr/lib/libSystem.B.dylib");
            let init = ane::init_with(api.as_ref().map_err(|e| *e));
            assert_eq!(super::npu_status_of(&init), ("unavailable", Some("symbol")));
        }
    }

    // Apple Silicon 실기(CYS_HW_STRICT=1): dlopen·구독이 실제로 되어 status "ok" + watts 숫자가 나와야 한다.
    // STRICT 가 없으면 npu 객체를 출력만 한다(CI VM 은 ANE 채널이 없을 수 있다).
    #[cfg(target_os = "macos")]
    #[test]
    fn e2_strict_npu_status_ok_on_apple_silicon() {
        let _ = super::snapshot();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let v = super::snapshot();
        println!("npu = {}", v["npu"]);
        if std::env::var("CYS_HW_STRICT").is_ok() {
            assert_eq!(v["npu"]["status"], "ok", "dlopen·구독 성공 기대: {v}");
            assert!(v["npu"].get("reason").is_none(), "{v}");
            assert!(v["npu"]["watts"].as_f64().is_some_and(|w| w >= 0.0), "{v}");
        }
    }
}
