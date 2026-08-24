// Graphite-zh 界面翻译引擎
// 架构：MutationObserver + DOM 文本/属性替换。字典见 zh-dict.js（先于本文件加载）。
// 设计约束：
//  1. 安全降级：字典未命中 → 原样英文，不修改 DOM。
//  2. 永不翻译用户输入：input/textarea 的 value、contenteditable、code/pre 不处理。
//  3. 防回环：替换写入的中文不会再命中字典键（键均为英文），无二次修改。
//  4. 性能：MutationObserver 批量回调 + 空闲调度，初始全量扫描一次。

(function () {
	"use strict";

	const dict = window.__ZH_DICT__ || {};
	if (Object.keys(dict).length === 0) return;

	// ---- 字典预处理：拆出精确表与模板正则表 ----
	const exact = new Map(); // "File" -> "文件"
	const templates = []; // [{ re: /^Zoom to (.+)%$/, out: "缩放到 $1" }]

	for (const [key, val] of Object.entries(dict)) {
		if (key.includes("{0}")) {
			const source = key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace("\\{0\\}", "(.+?)");
			const target = val.includes("{0}") ? val.replace("{0}", "$1") : val;
			templates.push({ re: new RegExp("^" + source + "$"), out: target });
		} else {
			exact.set(key, val);
		}
	}

	function translateLine(text) {
		if (!text) return null;
		const hit = exact.get(text);
		if (hit !== undefined) return hit;
		for (const t of templates) {
			const m = text.match(t.re);
			if (m) return text.replace(t.re, t.out);
		}
		return null;
	}

	function translate(raw) {
		if (!raw) return null;
		const text = raw.trim();
		if (!text || !/[\x20-\x7E]/.test(text)) return null; // 空或无 ASCII 可选项（纯中文/符号）直接跳过
		// 整串精确/模板匹配
		const whole = translateLine(text);
		if (whole !== null) return whole;
		// 多行文本（如 tooltip_description 经 .trim() 拼接的含 \n 整串）：按行拆分翻译，命中行替换、其余保留
		if (text.includes("\n")) {
			const lines = text.split("\n");
			let changed = false;
			const output = lines
				.map((line) => {
					const translated = translateLine(line.trim());
					if (translated !== null) {
						changed = true;
						return translated;
					}
					return line;
				})
				.join("\n");
			if (changed) return output;
		}
		return null;
	}

	// ---- DOM 遍历 ----
	const SKIP_TAGS = new Set(["SCRIPT", "STYLE", "CODE", "PRE", "NOSCRIPT", "TEXTAREA"]);
	const TRANSLATABLE_ATTRS = ["placeholder", "title", "aria-label"];

	function shouldSkipNode(node) {
		let p = node.parentNode;
		while (p && p !== document.body) {
			if (SKIP_TAGS.has(p.nodeName) || p.isContentEditable) return true;
			p = p.parentNode;
		}
		return false;
	}

	function processTextNode(node) {
		if (shouldSkipNode(node)) return;
		const translated = translate(node.nodeValue);
		// 保留原首尾空白（Svelte 条件块常产生空白文本节点，保持 DOM 稳定）
		if (translated !== null && translated !== node.nodeValue.trim()) {
			const lead = node.nodeValue.match(/^\s*/)[0];
			const tail = node.nodeValue.match(/\s*$/)[0];
			node.nodeValue = lead + translated + tail;
		}
	}

	function processElement(el) {
		if (SKIP_TAGS.has(el.nodeName) || el.isContentEditable) return;
		for (const attr of TRANSLATABLE_ATTRS) {
			if (!el.hasAttribute(attr)) continue;
			const translated = translate(el.getAttribute(attr));
			if (translated !== null) el.setAttribute(attr, translated);
		}
	}

	function scanSubtree(root) {
		if (root.nodeType === Node.TEXT_NODE) {
			processTextNode(root);
			return;
		}
		if (root.nodeType !== Node.ELEMENT_NODE && root.nodeType !== Node.DOCUMENT_NODE) return;
		const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT, {
			acceptNode: (n) => (SKIP_TAGS.has(n.nodeName) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
		});
		let node;
		while ((node = walker.nextNode())) {
			if (node.nodeType === Node.TEXT_NODE) processTextNode(node);
			else processElement(node);
		}
	}

	// ---- 持续监听（节点图、属性面板、对话框都是动态渲染）----
	const pending = new Set();
	let scheduled = false;

	function scheduleScan(target) {
		if (target.nodeType === Node.TEXT_NODE) pending.add(target);
		else if (target.nodeType === Node.ELEMENT_NODE || target.nodeType === Node.DOCUMENT_NODE) pending.add(target);

		if (scheduled) return;
		scheduled = true;
		const run = () => {
			scheduled = false;
			const batch = Array.from(pending);
			pending.clear();
			for (const t of batch) {
				if (!t.isConnected) continue;
				scanSubtree(t);
			}
		};
		if ("requestIdleCallback" in window) requestIdleCallback(run, { timeout: 200 });
		else setTimeout(run, 100);
	}

	const observer = new MutationObserver((mutations) => {
		for (const m of mutations) {
			if (m.type === "characterData") scheduleScan(m.target);
			else if (m.type === "childList") for (const n of m.addedNodes) scheduleScan(n);
			else if (m.type === "attributes") scheduleScan(m.target);
		}
	});

	function start() {
		scanSubtree(document.body);
		observer.observe(document.body, {
			childList: true,
			subtree: true,
			characterData: true,
			attributes: true,
			attributeFilter: TRANSLATABLE_ATTRS,
		});
	}

	if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
	else start();
})();
