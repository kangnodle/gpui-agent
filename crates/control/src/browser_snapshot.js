(textLimit, targetLimit) => {
    const cut = (text, limit) => Array.from(text || '').slice(0, limit).join('');
    const visible = el => {
        const style = getComputedStyle(el);
        return el.getClientRects().length && style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0';
    };
    const inView = el => {
        const rect = el.getBoundingClientRect();
        return rect.bottom > 0 && rect.right > 0 && rect.top < innerHeight && rect.left < innerWidth;
    };
    const all = Array.from(document.querySelectorAll('a[href],button,input,textarea,select,[role="button"],[role="link"],[role="textbox"]')).filter(visible);
    all.sort((a, b) => Number(inView(b)) - Number(inView(a)));
    const nodes = all.slice(0, targetLimit);
    globalThis.__gpuiAutomation = { document, nodes };
    const text = Array.from(document.body?.innerText || '');
    return {
        revision: '', url: cut(location.href, 8192), title: cut(document.title, 200),
        text: text.slice(0, textLimit).join(''), text_truncated: text.length > textLimit,
        targets_truncated: all.length > targetLimit,
        targets: nodes.map((el, id) => ({
            id, tag: el.tagName.toLowerCase(), kind: el.getAttribute('role') || el.type || el.tagName.toLowerCase(),
            label: cut(el.getAttribute('aria-label') || Array.from(el.labels || []).map(label => label.innerText).join(' ') || el.innerText || el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('alt') || el.tagName.toLowerCase(), 200)
        }))
    };
}
