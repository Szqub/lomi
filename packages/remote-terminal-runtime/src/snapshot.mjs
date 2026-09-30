// Exact xterm 6.0.0 adapter. No object graphs or executable state cross the wire.
export const SNAPSHOT_SCHEMA = "lomi-xterm-6-v1";
export const MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024;
export const SCROLLBACK = 200;
const MAX_TEXT = 65536;
const MAX_COMBINED = 16384;
export function installTerminalProfile(term) {
  // Fixed local palette; hyperlinks remain text. Graphics addons are absent.
  return [8, 4, 10, 11, 12, 104, 110, 111, 112].map((id) =>
    term.parser.registerOscHandler(id, () => true),
  );
}
export function assertModelBounds(term) {
  const c = core(term),
    h = c._inputHandler;
  let combinedBytes = 0;
  for (const b of [
    c._bufferService.buffers.normal,
    c._bufferService.buffers.alt,
  ])
    for (let i = 0; i < b.lines.length; i++)
      for (const v of Object.values(b.lines.get(i)._combined)) {
        if (
          v.length > MAX_COMBINED ||
          (combinedBytes += v.length) > 1024 * 1024
        )
          throw new Error("MODEL_LIMIT");
      }
  for (const v of [
    h._windowTitle,
    h._iconName,
    ...h._windowTitleStack,
    ...h._iconNameStack,
  ])
    if (v.length > MAX_TEXT) throw new Error("MODEL_LIMIT");
}

const fail = () => {
  throw new Error("INVALID_SNAPSHOT");
};
const uint = (v, max = 0xffffffff) =>
  Number.isInteger(v) && v >= 0 && v <= max ? v : fail();
const bool = (v) => (typeof v === "boolean" ? v : fail());
function record(v) {
  if (!v || typeof v !== "object" || Array.isArray(v)) fail();
  return v;
}
function keys(v, allowed) {
  record(v);
  if (Object.keys(v).some((k) => !allowed.includes(k))) fail();
}
function b64(bytes) {
  let s = "";
  for (let i = 0; i < bytes.length; i += 8192)
    s += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(s);
}
function unb64(s, length) {
  if (
    typeof s !== "string" ||
    s.length > Math.ceil(length / 3) * 4 ||
    !/^[A-Za-z0-9+/]*={0,2}$/.test(s)
  )
    fail();
  const v = atob(s);
  if (v.length !== length) fail();
  return Uint8Array.from(v, (c) => c.charCodeAt(0));
}
function core(term) {
  const c = term._core;
  if (
    !c?._bufferService?.buffers ||
    !c._inputHandler?._parser ||
    !c.coreService ||
    !c.coreMouseService
  )
    throw new Error("XTERM_VERSION_MISMATCH");
  return c;
}
export function safeBoundary(term) {
  const h = core(term)._inputHandler;
  return (
    h._parser.currentState === 0 && h._utf8Decoder.interim.every((v) => v === 0)
  );
}
function attr(a) {
  return { fg: a.fg >>> 0, bg: a.bg >>> 0, ext: a.extended.ext >>> 0 };
}
function charset(v) {
  return v ? Object.fromEntries(Object.entries(v)) : null;
}
function captureBuffer(b) {
  const lines = [];
  for (let i = 0; i < b.lines.length; i++) {
    const l = b.lines.get(i);
    const bytes = new Uint8Array(l.length * 12);
    const d = new DataView(bytes.buffer);
    for (let j = 0; j < l.length * 3; j++) d.setUint32(j * 4, l._data[j], true);
    const combined = {},
      extended = {};
    for (const [k, v] of Object.entries(l._combined))
      if (Number(k) < l.length && l.isCombined(Number(k))) combined[k] = v;
    for (const [k, v] of Object.entries(l._extendedAttrs))
      if (v && Number(k) < l.length && l.getBg(Number(k)) & 0x10000000)
        extended[k] = v.ext >>> 0;
    lines.push({
      cols: l.length,
      cells: b64(bytes),
      combined,
      extended,
      wrapped: l.isWrapped,
    });
  }
  return {
    lines,
    x: b.x,
    y: b.y,
    ybase: b.ybase,
    ydisp: b.ydisp,
    scrollTop: b.scrollTop,
    scrollBottom: b.scrollBottom,
    savedX: b.savedX,
    savedY: b.savedY,
    savedAttr: attr(b.savedCurAttrData),
    savedCharset: charset(b.savedCharset),
    tabs: Object.keys(b.tabs)
      .filter((k) => b.tabs[k])
      .map(Number),
  };
}
export function captureSnapshot(term) {
  if (!safeBoundary(term)) throw new Error("SNAPSHOT_PENDING");
  assertModelBounds(term);
  const c = core(term),
    h = c._inputHandler,
    bs = c._bufferService.buffers;
  const snapshot = {
    schema: SNAPSHOT_SCHEMA,
    convertEol: term.options.convertEol,
    cursorBlink: term.options.cursorBlink,
    cols: term.cols,
    rows: term.rows,
    normal: captureBuffer(bs.normal),
    alt: captureBuffer(bs.alt),
    active: bs.active === bs.alt ? "alt" : "normal",
    modes: { ...c.coreService.modes },
    dec: Object.fromEntries(
      Object.entries(c.coreService.decPrivateModes).filter(
        ([, v]) => v !== undefined,
      ),
    ),
    cursorHidden: c.coreService.isCursorHidden,
    cursorInitialized: c.coreService.isCursorInitialized,
    mouseProtocol: c.coreMouseService.activeProtocol,
    mouseEncoding: c.coreMouseService.activeEncoding,
    attr: attr(h._curAttrData),
    glevel: c._charsetService.glevel,
    activeCharset: charset(c._charsetService.charset),
    charsets: Array.from(c._charsetService._charsets, charset),
    join: uint(h._parser.precedingJoinState),
    title: h._windowTitle,
    icon: h._iconName,
    titleStack: [...h._windowTitleStack],
    iconStack: [...h._iconNameStack],
  };
  if (
    new TextEncoder().encode(JSON.stringify(snapshot)).byteLength >
    MAX_SNAPSHOT_BYTES
  )
    throw new Error("SNAPSHOT_LIMIT");
  validateSnapshot(snapshot);
  return snapshot;
}
function validateCharset(v) {
  if (v === null) return;
  record(v);
  if (Object.keys(v).length > 128) fail();
  for (const [k, x] of Object.entries(v))
    if (k.length !== 1 || typeof x !== "string" || x.length > 8) fail();
}
function validateAttr(v) {
  keys(v, ["fg", "bg", "ext"]);
  uint(v.fg);
  uint(v.bg);
  uint(v.ext);
}
const boolDec = [
  "applicationCursorKeys",
  "applicationKeypad",
  "bracketedPasteMode",
  "origin",
  "reverseWraparound",
  "sendFocus",
  "synchronizedOutput",
  "wraparound",
];
export function validateSnapshot(s) {
  keys(s, [
    "schema",
    "convertEol",
    "cursorBlink",
    "cols",
    "rows",
    "normal",
    "alt",
    "active",
    "modes",
    "dec",
    "cursorHidden",
    "cursorInitialized",
    "mouseProtocol",
    "mouseEncoding",
    "attr",
    "glevel",
    "activeCharset",
    "charsets",
    "join",
    "title",
    "icon",
    "titleStack",
    "iconStack",
  ]);
  if (s.schema !== SNAPSHOT_SCHEMA) fail();
  bool(s.convertEol);
  bool(s.cursorBlink);
  uint(s.cols, 500);
  uint(s.rows, 500);
  if (
    s.cols < 2 ||
    s.rows < 1 ||
    new TextEncoder().encode(JSON.stringify(s)).byteLength > MAX_SNAPSHOT_BYTES
  )
    fail();
  if (!["normal", "alt"].includes(s.active)) fail();
  for (const [name, b] of [
    ["normal", s.normal],
    ["alt", s.alt],
  ]) {
    keys(b, [
      "lines",
      "x",
      "y",
      "ybase",
      "ydisp",
      "scrollTop",
      "scrollBottom",
      "savedX",
      "savedY",
      "savedAttr",
      "savedCharset",
      "tabs",
    ]);
    if (
      !Array.isArray(b.lines) ||
      b.lines.length > s.rows + (name === "normal" ? SCROLLBACK : 0) ||
      ((name === "normal" || s.active === name || b.lines.length) &&
        b.lines.length < s.rows)
    )
      fail();
    uint(b.x, s.cols);
    uint(b.y, s.rows - 1);
    uint(b.ybase, SCROLLBACK);
    uint(b.ydisp, b.ybase);
    if (b.lines.length && b.ybase + s.rows > b.lines.length) fail();
    uint(b.scrollTop, s.rows - 1);
    uint(b.scrollBottom, s.rows - 1);
    if (b.scrollTop > b.scrollBottom) fail();
    uint(b.savedX, 500);
    uint(b.savedY, SCROLLBACK + s.rows);
    validateAttr(b.savedAttr);
    validateCharset(b.savedCharset);
    if (!Array.isArray(b.tabs) || b.tabs.length > 500) fail();
    b.tabs.forEach((v) => uint(v, 499));
    for (const l of b.lines) {
      keys(l, ["cols", "cells", "combined", "extended", "wrapped"]);
      uint(l.cols, 500);
      if (l.cols < 2) fail();
      bool(l.wrapped);
      unb64(l.cells, l.cols * 12);
      record(l.combined);
      record(l.extended);
      if (
        Object.keys(l.combined).length > l.cols ||
        Object.keys(l.extended).length > l.cols
      )
        fail();
      for (const [k, v] of Object.entries(l.combined)) {
        if (
          !/^\d{1,3}$/.test(k) ||
          uint(+k, l.cols - 1) !== +k ||
          typeof v !== "string" ||
          v.length > MAX_COMBINED
        )
          fail();
      }
      for (const [k, v] of Object.entries(l.extended)) {
        if (!/^\d{1,3}$/.test(k)) fail();
        uint(+k, l.cols - 1);
        uint(v);
      }
      const packed = unb64(l.cells, l.cols * 12),
        dv = new DataView(packed.buffer);
      for (let col = 0; col < l.cols; col++) {
        const content = dv.getUint32(col * 12, true),
          bg = dv.getUint32(col * 12 + 8, true);
        if (content >>> 22 > 2) fail();
        if (content & 0x200000) {
          if (typeof l.combined[col] !== "string") fail();
        } else if ((content & 0x1fffff) > 0x10ffff) fail();
        if (bg & 0x10000000 && l.extended[col] === undefined) fail();
      }
    }
  }
  keys(s.modes, ["insertMode"]);
  bool(s.modes.insertMode);
  keys(s.dec, [...boolDec, "cursorBlink", "cursorStyle"]);
  for (const k of boolDec) bool(s.dec[k]);
  if (s.dec.cursorBlink !== undefined) bool(s.dec.cursorBlink);
  if (
    s.dec.cursorStyle !== undefined &&
    !["block", "underline", "bar"].includes(s.dec.cursorStyle)
  )
    fail();
  bool(s.cursorHidden);
  bool(s.cursorInitialized);
  if (
    !["NONE", "X10", "VT200", "DRAG", "ANY"].includes(s.mouseProtocol) ||
    !["DEFAULT", "SGR", "SGR_PIXELS"].includes(s.mouseEncoding)
  )
    fail();
  validateAttr(s.attr);
  uint(s.glevel, 3);
  validateCharset(s.activeCharset);
  uint(s.join);
  if (!Array.isArray(s.charsets) || s.charsets.length > 4) fail();
  s.charsets.forEach(validateCharset);
  for (const k of ["title", "icon"])
    if (typeof s[k] !== "string" || s[k].length > MAX_TEXT) fail();
  for (const k of ["titleStack", "iconStack"]) {
    if (!Array.isArray(s[k]) || s[k].length > 10) fail();
    for (const v of s[k])
      if (typeof v !== "string" || v.length > MAX_TEXT) fail();
  }
  if (
    new TextEncoder().encode(JSON.stringify(s)).byteLength > MAX_SNAPSHOT_BYTES
  )
    fail();
  return s;
}
function setAttr(a, v) {
  a.fg = v.fg;
  a.bg = v.bg;
  a.extended.ext = v.ext;
  a.extended.urlId = 0;
}
function restoreBuffer(b, v, cols) {
  b.lines.length = 0;
  for (const l of v.lines) {
    const line = b.getBlankLine();
    line.resize(l.cols, b.getNullCell());
    const bytes = unb64(l.cells, l.cols * 12),
      d = new DataView(bytes.buffer);
    for (let j = 0; j < l.cols * 3; j++)
      line._data[j] = d.getUint32(j * 4, true);
    line._combined = Object.assign(Object.create(null), l.combined);
    line._extendedAttrs = Object.create(null);
    for (const [k, ext] of Object.entries(l.extended)) {
      const a = b.savedCurAttrData.extended.clone();
      a.ext = ext;
      a.urlId = 0;
      line._extendedAttrs[k] = a;
    }
    line.isWrapped = l.wrapped;
    b.lines.push(line);
  }
  for (const k of [
    "x",
    "y",
    "ybase",
    "ydisp",
    "scrollTop",
    "scrollBottom",
    "savedX",
    "savedY",
  ])
    b[k] = v[k];
  setAttr(b.savedCurAttrData, v.savedAttr);
  b.savedCharset =
    v.savedCharset === null
      ? undefined
      : Object.assign(Object.create(null), v.savedCharset);
  b.tabs = Object.fromEntries(v.tabs.map((k) => [k, true]));
}
export function restoreSnapshot(term, input) {
  const s = validateSnapshot(input);
  term.reset();
  term.resize(s.cols, s.rows);
  term.options.convertEol = s.convertEol;
  term.options.cursorBlink = s.cursorBlink;
  const c = core(term),
    bs = c._bufferService.buffers;
  if (s.active === "alt") bs.activateAltBuffer();
  else bs.activateNormalBuffer();
  restoreBuffer(bs.normal, s.normal, s.cols);
  restoreBuffer(bs.alt, s.alt, s.cols);
  c.coreService.modes = { ...s.modes };
  c.coreService.decPrivateModes = { ...s.dec };
  c.coreService.isCursorHidden = s.cursorHidden;
  c.coreService.isCursorInitialized = s.cursorInitialized;
  c.coreMouseService.activeProtocol = s.mouseProtocol;
  c.coreMouseService.activeEncoding = s.mouseEncoding;
  setAttr(c._inputHandler._curAttrData, s.attr);
  c._charsetService._charsets = s.charsets.map((v) =>
    v === null ? undefined : Object.assign(Object.create(null), v),
  );
  c._charsetService.setgLevel(s.glevel);
  c._charsetService.charset =
    s.activeCharset === null
      ? undefined
      : Object.assign(Object.create(null), s.activeCharset);
  c._inputHandler._parser.precedingJoinState = s.join;
  c._inputHandler._windowTitle = s.title;
  c._inputHandler._iconName = s.icon;
  c._inputHandler._windowTitleStack = [...s.titleStack];
  c._inputHandler._iconNameStack = [...s.iconStack];
  c._inputHandler._eraseAttrDataInternal.fg = 0;
  c._inputHandler._eraseAttrDataInternal.bg = 0;
  c._inputHandler._eraseAttrDataInternal.extended.ext = 0;
  c._inputHandler._eraseAttrDataInternal.extended.urlId = 0;
  c._viewport?.queueSync();
  term.refresh?.(0, s.rows - 1);
  return s;
}
