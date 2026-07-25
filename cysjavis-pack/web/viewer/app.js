/* 자비스 뷰어 앱 — md/diff 렌더 · 라이브 리로드 · 파일목록 · 테마 전파.
   전부 로컬 vendored(CDN 런타임 로드 없음). BASE 는 URL 토큰 prefix 에서 유도. */
(function () {
  "use strict";

  // /<token>/app/... 에서 <token> 추출 → api 베이스 구성.
  var seg = location.pathname.split("/").filter(Boolean);
  var TOKEN = seg[0] || "";
  var API = "/" + TOKEN + "/api";

  var $ = function (id) { return document.getElementById(id); };
  var content = $("content");
  var docTitle = $("doc-title");
  var statusEl = $("status");
  var liveBadge = $("live-badge");
  var fileList = $("file-list");
  var sidePath = $("side-path");

  var watchSrc = null;    // 현재 SSE
  var mermaidReady = null; // 지연 로드 프로미스

  // ---------------------------------------------------------------- 유틸
  function setStatus(t) { statusEl.textContent = t || ""; }

  function esc(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;",
               '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  // marked v12.0.2 — 신뢰 md 뷰어는 산문의 raw HTML(꺾쇠 제네릭 `<Option>`·`<T>`·
  // `<Stream>` 등 태그 유사 텍스트)을 해석하지 말고 문자 그대로 표시해야 한다. 미해석 시
  // marked 가 `<Option>` 을 인라인 raw HTML 로 통과 → `<option>` 요소로 파싱돼 UA 기본
  // 스타일상 안 보이고 이후 텍스트가 요소로 흡수돼 사라진다(실측: `RwLock<Option>…` 잘림).
  // 수정: renderer.html 오버라이드로 raw HTML 토큰을 이스케이프. 이 marked 버전은 블록·인라인
  // raw HTML 을 모두 renderer.html 로 보낸다(파서: 블록 `renderer.html(e.text,e.block)` ·
  // 인라인 `t.html(e.text)`). 코드펜스(```)·인라인코드(`)는 renderer.code/codespan 경유라
  // 이 오버라이드의 영향을 전혀 받지 않아 원문(이미 marked 가 엔티티 이스케이프)이 보존된다.
  // sanitize()의 보안 제거 목록·스킴 필터는 심층방어로 그대로 유지(약화 금지).
  if (window.marked && typeof window.marked.use === "function") {
    window.marked.use({ renderer: { html: function (h) { return esc(h); } } });
  }

  function apiURL(ep, params) {
    var u = API + ep + "?";
    var kv = [];
    for (var k in params) {
      kv.push(encodeURIComponent(k) + "=" + encodeURIComponent(params[k]));
    }
    return u + kv.join("&");
  }

  function showError(msg) {
    content.innerHTML = '<div class="errbox">' + esc(msg) + "</div>";
  }

  // marked 출력 sanitize — 신뢰 경계 내부 파일이지만 script/iframe/on*/javascript:
  // 제거로 방어선 유지(§XSS). DOMParser(스크립팅 OFF)로 파싱 후 위험 노드·속성 제거.
  // noscript/svg/math/template 은 스크립팅 OFF 파싱에선 무해해 보여도 라이브
  // innerHTML 재삽입 시 재파싱돼 mXSS 로 되살아날 수 있어 함께 제거한다(심층방어).
  function sanitize(html) {
    var doc = new DOMParser().parseFromString(html, "text/html");
    var bad = doc.querySelectorAll(
      "script,iframe,object,embed,link,meta,form,base,style," +
      "noscript,svg,math,template");
    for (var i = 0; i < bad.length; i++) bad[i].remove();
    var all = doc.body.querySelectorAll("*");
    for (var j = 0; j < all.length; j++) {
      var el = all[j];
      var attrs = Array.prototype.slice.call(el.attributes);
      for (var a = 0; a < attrs.length; a++) {
        var name = attrs[a].name, val = attrs[a].value || "";
        if (/^on/i.test(name)) { el.removeAttribute(name); continue; }
        if (name === "href" || name === "src" || name === "xlink:href") {
          // 스킴 판정 전 제어문자(U+0000-U+0020: 탭·개행·CR·NUL 포함) 제거 정규화 —
          // 브라우저는 URL 에서 이들을 무시하므로 `java<TAB>script:` 처럼 스킴 중간에
          // 끼운 제어문자로 필터를 우회하는 벡터를 차단한다.
          var scheme = val.replace(/[\u0000-\u0020]+/g, "");
          if (/^(javascript|data|vbscript):/i.test(scheme)) {
            el.removeAttribute(name);
          }
        }
      }
    }
    return doc.body.innerHTML;
  }

  // ---------------------------------------------------------------- mermaid 지연 로드
  function ensureMermaid() {
    if (mermaidReady) return mermaidReady;
    mermaidReady = new Promise(function (resolve, reject) {
      var s = document.createElement("script");
      s.src = "./vendor/mermaid.min.js";
      s.onload = function () {
        try {
          window.mermaid.initialize({
            startOnLoad: false,
            securityLevel: "strict",
            theme: isDark() ? "dark" : "default",
          });
          resolve(window.mermaid);
        } catch (e) { reject(e); }
      };
      s.onerror = function () { reject(new Error("mermaid load fail")); };
      document.head.appendChild(s);
    });
    return mermaidReady;
  }

  // ---------------------------------------------------------------- md 렌더
  function renderMarkdown(text) {
    var html = window.marked.parse(text, { breaks: false, gfm: true });
    var wrap = document.createElement("div");
    wrap.className = "md";
    wrap.innerHTML = sanitize(html);
    content.innerHTML = "";
    content.appendChild(wrap);

    // 코드블록: mermaid 펜스는 다이어그램, 나머지는 hljs.
    var blocks = wrap.querySelectorAll("pre code");
    var mermaidNodes = [];
    for (var i = 0; i < blocks.length; i++) {
      var code = blocks[i];
      if (/\blanguage-mermaid\b/.test(code.className)) {
        var div = document.createElement("div");
        div.className = "mermaid";
        div.textContent = code.textContent;
        code.parentNode.replaceWith(div);
        mermaidNodes.push(div);
      } else if (window.hljs) {
        try { window.hljs.highlightElement(code); } catch (e) {}
      }
    }
    if (mermaidNodes.length) {
      ensureMermaid().then(function (m) {
        m.run({ nodes: mermaidNodes }).catch(function () {});
      }).catch(function () {
        for (var k = 0; k < mermaidNodes.length; k++) {
          mermaidNodes[k].textContent = "[mermaid 렌더 실패]";
        }
      });
    }
  }

  // ---------------------------------------------------------------- diff 렌더
  function renderDiff(text) {
    var wrap = document.createElement("div");
    wrap.className = "diff";
    var lines = text.split("\n");
    var tbl = null, ln = 0;
    function newFile(name) {
      var h = document.createElement("div");
      h.className = "file-head";
      h.textContent = name;
      wrap.appendChild(h);
      tbl = document.createElement("table");
      wrap.appendChild(tbl);
    }
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];
      if (line.indexOf("diff --git") === 0) {
        newFile(line.replace("diff --git ", ""));
        continue;
      }
      if (!tbl) newFile("(diff)");
      var cls = "";
      if (line[0] === "+" && line.indexOf("+++") !== 0) cls = "add";
      else if (line[0] === "-" && line.indexOf("---") !== 0) cls = "del";
      else if (line.indexOf("@@") === 0) cls = "hunk";
      else if (line.indexOf("+++") === 0 || line.indexOf("---") === 0 ||
               line.indexOf("index ") === 0) cls = "meta";
      var tr = document.createElement("tr");
      tr.className = cls;
      var td1 = document.createElement("td");
      td1.className = "ln";
      td1.textContent = cls && cls !== "meta" ? "" : "";
      var td2 = document.createElement("td");
      td2.className = "code";
      td2.textContent = line || " ";
      tr.appendChild(td1);
      tr.appendChild(td2);
      tbl.appendChild(tr);
    }
    if (!lines.join("").trim()) {
      wrap.innerHTML = '<div class="empty">변경 없음 (빈 diff)</div>';
    }
    content.innerHTML = "";
    content.appendChild(wrap);
  }

  // ---------------------------------------------------------------- 코드 렌더
  // 확장자 → hljs 언어. 미매핑·미지 확장자는 plaintext(hljs auto 금지 — 예측가능).
  // 매핑 대상은 vendored highlight.min.js(v11.9.0) 에 등록된 문법으로 한정하고,
  // 없으면 renderCode 진입 시 getLanguage 로 plaintext 폴백한다(심층 안전).
  var LANG_MAP = {
    py: "python", rs: "rust", ts: "typescript", tsx: "typescript",
    js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "javascript",
    json: "json", toml: "ini", ini: "ini",
    yaml: "yaml", yml: "yaml",
    sh: "bash", bash: "bash", zsh: "bash",
    html: "xml", htm: "xml", xml: "xml",
    css: "css", scss: "scss", less: "less",
    sql: "sql", go: "go",
    c: "c", h: "c", cpp: "cpp", cc: "cpp", cxx: "cpp", hpp: "cpp",
    java: "java", rb: "ruby", php: "php", swift: "swift",
    kt: "kotlin", kts: "kotlin", lua: "lua", pl: "perl", r: "r",
    txt: "plaintext", log: "plaintext", env: "plaintext", conf: "plaintext"
  };

  function fileExt(name) {
    var i = name.lastIndexOf(".");
    return i >= 0 ? name.slice(i + 1).toLowerCase() : "";
  }

  // 확장자로 렌더 종류 분기: md/markdown → 마크다운, diff/patch → diff, 그 외 → 코드.
  function fileKind(name) {
    var e = fileExt(name);
    if (e === "md" || e === "markdown") return "md";
    if (e === "diff" || e === "patch") return "diff";
    return "code";
  }

  // 코드 파일 단독 뷰어: 원문을 textContent 로만 주입(innerHTML 금지)한 뒤 hljs 로 색칠.
  // marked·sanitize 를 경유하지 않는다 — hljs 는 DOM 텍스트 노드만 하이라이트하므로
  // raw HTML 이 실행 컨텍스트로 들어갈 표면이 없다(★XSS 없음).
  function renderCode(text, name) {
    var lang = LANG_MAP[fileExt(name)] || "plaintext";
    // vendored hljs 에 해당 문법이 없으면 plaintext 폴백(등록 언어만 신뢰).
    if (lang !== "plaintext" && window.hljs &&
        typeof window.hljs.getLanguage === "function" &&
        !window.hljs.getLanguage(lang)) {
      lang = "plaintext";
    }
    var wrap = document.createElement("div");
    wrap.className = "code-view";
    var pre = document.createElement("pre");
    var code = document.createElement("code");
    code.className = "language-" + lang;
    code.textContent = text;          // ★원문 안전 삽입(innerHTML 아님)
    pre.appendChild(code);
    wrap.appendChild(pre);
    content.innerHTML = "";
    content.appendChild(wrap);
    if (window.hljs) {
      try { window.hljs.highlightElement(code); } catch (e) {}
    }
  }

  // ---------------------------------------------------------------- 데이터 로드
  function loadFile(path) {
    setStatus("불러오는 중…");
    fetch(apiURL("/file", { path: path }))
      .then(function (r) { return r.json(); })
      .then(function (j) {
        if (!j.ok) { showError("파일 오류: " + (j.error || "")); setStatus(""); return; }
        var name = path.split("/").pop();
        docTitle.textContent = name;
        var kind = fileKind(name);
        if (kind === "md") {
          renderMarkdown(j.content + (j.truncated ?
            "\n\n> **[2MB 초과 — 이후 절단됨]**" : ""));
        } else if (kind === "diff") {
          renderDiff(j.content + (j.truncated ?
            "\n[2MB 초과 — 이후 절단됨]" : ""));
        } else {
          renderCode(j.content + (j.truncated ?
            "\n[2MB 초과 — 이후 절단됨]" : ""), name);
        }
        setStatus(j.truncated ? "절단됨 · " + j.size + "B" : j.size + "B");
        subscribeWatch(path);
      })
      .catch(function (e) { showError("네트워크 오류: " + e); setStatus(""); });
  }

  function loadDiff(repo, base) {
    setStatus("git diff…");
    docTitle.textContent = "diff " + base + " · " + repo.split("/").pop();
    fetch(apiURL("/diff", { repo: repo, base: base }))
      .then(function (r) { return r.json(); })
      .then(function (j) {
        if (!j.ok) {
          showError("diff 오류: " + (j.error || "") +
            (j.stderr ? "\n" + j.stderr : ""));
          setStatus(""); return;
        }
        renderDiff(j.diff);
        setStatus("diff " + base);
      })
      .catch(function (e) { showError("네트워크 오류: " + e); setStatus(""); });
  }

  // ---------------------------------------------------------------- 라이브 리로드
  function subscribeWatch(path) {
    if (watchSrc) { watchSrc.close(); watchSrc = null; }
    liveBadge.hidden = false;
    watchSrc = new EventSource(apiURL("/watch", { path: path }));
    watchSrc.onmessage = function (ev) {
      try {
        var d = JSON.parse(ev.data);
        if (d.type === "change") {
          setStatus("변경 감지 · 리로드");
          loadFile(path);  // 재구독은 loadFile 내부에서 갱신
        }
      } catch (e) {}
    };
    watchSrc.onerror = function () { liveBadge.hidden = true; };
  }

  // ---------------------------------------------------------------- 파일 목록
  function listDir(dir) {
    sidePath.textContent = dir;
    fetch(apiURL("/list", { path: dir }))
      .then(function (r) { return r.json(); })
      .then(function (j) {
        fileList.innerHTML = "";
        if (!j.ok) {
          var li = document.createElement("li");
          li.textContent = "(" + (j.error || "목록 불가") + ")";
          fileList.appendChild(li);
          return;
        }
        // 상위로 이동
        var up = document.createElement("li");
        up.className = "dir";
        up.innerHTML = '<span class="ic">↰</span>..';
        up.onclick = function () { listDir(parentDir(dir)); };
        fileList.appendChild(up);
        j.entries.forEach(function (e) {
          var li = document.createElement("li");
          var isDir = e.type === "dir";
          li.className = isDir ? "dir" : "";
          li.innerHTML = '<span class="ic">' + (isDir ? "📁" : "📄") +
            "</span>" + esc(e.name);
          var full = dir.replace(/\/$/, "") + "/" + e.name;
          li.onclick = isDir
            ? function () { listDir(full); }
            : function () { openPath(full); };
          fileList.appendChild(li);
        });
      })
      .catch(function () {});
  }

  function parentDir(p) {
    var q = p.replace(/\/+$/, "");
    var i = q.lastIndexOf("/");
    return i > 0 ? q.slice(0, i) : q;
  }

  function openPath(path) {
    // 확장자로 분기 렌더(md·diff·코드). loadFile 이 fileKind 로 판별. URL 갱신.
    var u = new URL(location.href);
    u.searchParams.delete("diff");
    u.searchParams.delete("base");
    u.searchParams.set("path", path);
    history.pushState({}, "", u);
    loadFile(path);
    listDir(parentDir(path));
  }

  // ---------------------------------------------------------------- 테마 전파
  function isDark() {
    return document.body.getAttribute("data-theme") !== "light";
  }
  function applyTheme(dark) {
    document.body.setAttribute("data-theme", dark ? "dark" : "light");
    $("hljs-dark").disabled = !dark;
    $("hljs-light").disabled = dark;
  }
  function luminance(hex) {
    var m = /^#?([0-9a-f]{6})$/i.exec((hex || "").trim());
    if (!m) return null;
    var n = parseInt(m[1], 16);
    var r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  }
  // cys-terminal 웹 pane → postMessage 로 테마 변수 수신(§14).
  window.addEventListener("message", function (ev) {
    var d = ev.data;
    if (!d || d.type !== "cys-theme" || !d.vars) return;
    var root = document.documentElement;
    for (var k in d.vars) {
      if (/^--[\w-]+$/.test(k)) root.style.setProperty(k, d.vars[k]);
    }
    var lum = luminance(d.vars["--bg"]);
    if (lum !== null) applyTheme(lum < 0.5);
  });

  // ---------------------------------------------------------------- 부팅
  function boot() {
    // 시스템 prefers-color-scheme 폴백(메시지 오기 전 기본).
    if (window.matchMedia &&
        window.matchMedia("(prefers-color-scheme: light)").matches) {
      applyTheme(false);
    }
    $("side-refresh").onclick = function () {
      listDir(sidePath.textContent || firstRoot());
    };
    window.addEventListener("popstate", route);
    route();
  }

  function firstRoot() { return "~/Desktop/CYSjavis/_round"; }

  function route() {
    var q = new URLSearchParams(location.search);
    var path = q.get("path");
    var diff = q.get("diff");
    if (diff) {
      loadDiff(diff, q.get("base") || "HEAD");
      listDir(diff);
    } else if (path) {
      loadFile(path);
      listDir(parentDir(path));
    } else {
      listDir(firstRoot());
    }
  }

  boot();
})();
