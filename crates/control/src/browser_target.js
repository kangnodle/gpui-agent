(id, text) => {
    const state = globalThis.__gpuiAutomation;
    const el = state?.nodes[id];
    if (!el || state.document !== document || !el.isConnected || el.disabled || el.closest('[inert]') || el.getAttribute('aria-disabled') === 'true') return { error: true };
    const style = getComputedStyle(el);
    if (!el.getClientRects().length || style.display === 'none' || style.visibility !== 'visible' || style.opacity === '0' || el.type === 'file') return { error: true };
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const rect = el.getBoundingClientRect();
    const x = Math.floor((Math.max(0, rect.left) + Math.min(innerWidth, rect.right)) / 2);
    const y = Math.floor((Math.max(0, rect.top) + Math.min(innerHeight, rect.bottom)) / 2);
    if (rect.width <= 0 || rect.height <= 0 || !el.contains(document.elementFromPoint(x, y))) return { error: true };
    if (text === null) return { x, y };
    if (el.readOnly || !(el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && ['text','search','email','url','tel','password','number'].includes(el.type)))) return { error: true };
    el.focus();
    const prototype = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(el, text);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return { ok: true };
}
