// Minimal hyperscript helper — no framework. `h(tag, attrs, ...children)`
// builds a real DOM node. Attr values of `null`/`undefined` are skipped;
// `boolean` toggles the attribute; functions attach event listeners.

type AttrValue =
  | string
  | boolean
  | null
  | undefined
  | EventListenerOrEventListenerObject;

type Child = Node | string | null | undefined | (Node | string | null | undefined)[];

function appendChildren(node: Node, children: Child[]): void {
  for (const c of children) {
    if (c == null) continue;
    if (Array.isArray(c)) {
      appendChildren(node, c);
      continue;
    }
    node.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
  }
}

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs?: Record<string, AttrValue> | null,
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (attrs) {
    for (const [key, val] of Object.entries(attrs)) {
      if (val == null || val === false) continue;
      if (key === "class" || key === "className") {
        if (typeof val === "string") node.className = val;
      } else if (key === "style" && typeof val === "string") {
        node.setAttribute("style", val);
      } else if (typeof val === "function") {
        node.addEventListener(key.slice(2).toLowerCase(), val as EventListener);
      } else if (val === true) {
        node.setAttribute(key, "");
      } else {
        node.setAttribute(key, String(val));
      }
    }
  }
  appendChildren(node, children);
  return node;
}

export function clear(node: Node): void {
  while (node.firstChild) node.removeChild(node.firstChild);
}

// Render a JSON value as a pretty-printed string, truncated for display.
export function fmtJson(v: unknown, max = 4000): string {
  let s: string;
  try {
    s = typeof v === "string" ? v : JSON.stringify(v, null, 2);
  } catch {
    s = String(v);
  }
  return s.length > max ? s.slice(0, max) + `\n… (${s.length - max} more chars)` : s;
}

// Short hash form for display: first 12 chars.
export function shortHash(h: string): string {
  return h.length > 12 ? h.slice(0, 12) + "…" : h;
}
