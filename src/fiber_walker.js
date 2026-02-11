(() => {
    const rootIds = ['root', 'app', '__next', 'main', '__nuxt'];
    let fiberRoot = null;

    for (const id of rootIds) {
      const el = document.getElementById(id);
      if (!el) continue;
      const key = Object.keys(el).find(k => k.startsWith('__reactFiber$') || k.startsWith('__reactInternalInstance$'));
      if (key) { fiberRoot = el[key]; break; }
    }

    if (!fiberRoot) {
      const walk = (el) => {
        const key = Object.keys(el).find(k => k.startsWith('__reactFiber$') || k.startsWith('__reactInternalInstance$'));
        if (key) return el[key];
        for (const child of el.children) {
          const found = walk(child);
          if (found) return found;
        }
        return null;
      };
      fiberRoot = walk(document.body);
    }

    if (!fiberRoot) return { found: false, tree: [], allMinified: false };

    let root = fiberRoot;
    while (root.return) root = root.return;

    document.querySelectorAll('[data-ab-ref]').forEach(el => el.removeAttribute('data-ab-ref'));

    let refCounter = 0;
    let totalComponents = 0;
    let minifiedComponents = 0;

    const INTERACTIVE_TAGS = new Set(['a', 'button', 'input', 'select', 'textarea', 'details', 'summary']);
    const SKIP_PROPS = new Set(['children', 'className', 'style', 'key', 'ref', 'dangerouslySetInnerHTML', '__css', 'sx', 'css']);
    const STYLING_PROPS = new Set([
      'alignItems', 'justifyContent', 'direction', 'flexDirection', 'display', 'position',
      'width', 'height', 'minWidth', 'minHeight', 'maxWidth', 'maxHeight', 'w', 'h',
      'margin', 'padding', 'p', 'px', 'py', 'pt', 'pb', 'pl', 'pr', 'm', 'mx', 'my', 'mt', 'mb', 'ml', 'mr',
      'paddingTop', 'paddingBottom', 'paddingLeft', 'paddingRight', 'paddingX', 'paddingY', 'paddingStart', 'paddingEnd',
      'gap', 'columnGap', 'rowGap', 'flex', 'flexGrow', 'flexShrink', 'flexBasis', 'wrap', 'flexWrap',
      'top', 'bottom', 'left', 'right', 'zIndex', 'overflow',
      'color', 'background', 'backgroundColor', 'bg', 'borderColor', 'borderBottom', 'borderTop',
      'border', 'borderRadius', 'borderWidth', 'boxShadow', 'outline',
      'fontSize', 'fontWeight', 'lineHeight', 'textColor', 'textAlign', 'letterSpacing',
      'opacity', 'transition', 'animation', 'transform', 'pointerEvents', 'cursor',
      '_hover', '_focus', '_active', '_focusVisible', '_placeholder', '_peerFocus',
      'data-theme', 'data-group', 'data-peer',
      'boxSizing', 'textDecoration', 'whiteSpace', 'wordBreak', 'overflowWrap',
    ]);
    const SKIP_COMPONENTS = new Set([
      'Styled(div)', 'Styled(button)', 'Styled(input)', 'Styled(select)', 'Styled(a)', 'Styled(span)', 'Styled(img)',
      'Insertion4', 'EmotionGlobal', 'CSSVars', 'CSSReset', 'GlobalStyle',
    ]);

    function getComponentName(fiber) {
      if (!fiber.type) return null;
      if (typeof fiber.type === 'string') return null;
      return fiber.type.displayName || fiber.type.name || null;
    }

    function isMinified(name) {
      return name && name.length <= 2 && /^[a-z]/.test(name);
    }

    function filterProps(props, isHost) {
      if (!props) return {};
      const result = {};
      for (const [key, value] of Object.entries(props)) {
        if (SKIP_PROPS.has(key)) continue;
        if (typeof value === 'function') continue;
        if (isHost && key.startsWith('on')) continue;
        if (key.startsWith('data-sentry-')) continue;
        if (key.startsWith('data-') && !key.startsWith('data-testid') && !key.startsWith('data-gc-')) continue;
        if (STYLING_PROPS.has(key)) continue;
        if (typeof value === 'string') {
          result[key] = value.length > 80 ? value.slice(0, 80) + '...' : value;
        } else if (typeof value === 'number' || typeof value === 'boolean') result[key] = value;
        else if (value === null) result[key] = null;
        else if (value !== undefined) result[key] = '{...}';
      }
      return result;
    }

    function getHostAttrs(domNode) {
      if (!domNode || !domNode.getAttribute) return {};
      const attrs = {};
      for (const name of ['href', 'src', 'type', 'placeholder', 'alt', 'title', 'name', 'value', 'action', 'method']) {
        const val = domNode.getAttribute(name);
        if (val) attrs[name] = val;
      }
      if (domNode.dataset) {
        for (const [k, v] of Object.entries(domNode.dataset)) {
          if (k === 'abRef' || k.startsWith('sentry') || k === 'theme' || k === 'group' || k === 'peer') continue;
          const attrName = 'data-' + k.replace(/([A-Z])/g, '-$1').toLowerCase();
          attrs[attrName] = v;
        }
      }
      return attrs;
    }

    function getAriaInfo(domNode) {
      if (!domNode) return {};
      const role = domNode.getAttribute('role') || domNode.tagName?.toLowerCase();
      const ariaLabel = domNode.getAttribute('aria-label');
      const textContent = (INTERACTIVE_TAGS.has(domNode.tagName?.toLowerCase()) && !ariaLabel)
        ? (domNode.textContent?.trim().slice(0, 80) || undefined)
        : undefined;
      return { role, ariaName: ariaLabel || textContent || undefined };
    }

    function shouldIncludeHost(tag, domNode) {
      if (INTERACTIVE_TAGS.has(tag)) return true;
      if (domNode?.getAttribute('role')) return true;
      if (domNode?.getAttribute('aria-label')) return true;
      if (tag === 'img' || tag === 'video' || tag === 'audio' || tag === 'svg') return true;
      if (/^h[1-6]$/.test(tag)) return true;
      if (tag === 'label') return true;
      return false;
    }

    function processFiber(fiber, depth) {
      if (!fiber) return [];
      const results = [];
      let current = fiber;
      while (current) {
        results.push(...processOneFiber(current, depth));
        current = current.sibling;
      }
      return results;
    }

    function processOneFiber(fiber, depth) {
      if (!fiber || depth > (globalThis.__MAX_DEPTH || 50)) return [];
      const isHost = typeof fiber.type === 'string';
      const tag = isHost ? fiber.type : null;
      const componentName = isHost ? null : getComponentName(fiber);
      const domNode = fiber.stateNode instanceof HTMLElement ? fiber.stateNode : null;
      const childNodes = processFiber(fiber.child, depth + 1);

      if (isHost && tag) {
        if (!shouldIncludeHost(tag, domNode)) return childNodes;
        const refId = 'e' + (++refCounter);
        if (domNode) domNode.setAttribute('data-ab-ref', refId);
        const ariaInfo = getAriaInfo(domNode);
        const htmlAttrs = getHostAttrs(domNode);
        const props = filterProps(fiber.memoizedProps, true);
        return [{ name: tag, isComponent: false, props, ref: refId, role: ariaInfo.role, ariaName: ariaInfo.ariaName, tag, htmlAttrs: Object.keys(htmlAttrs).length > 0 ? htmlAttrs : undefined, children: childNodes }];
      }

      if (componentName) {
        totalComponents++;
        if (isMinified(componentName)) { minifiedComponents++; return childNodes; }
        if (SKIP_COMPONENTS.has(componentName)) return childNodes;
        const props = filterProps(fiber.memoizedProps, false);
        return [{ name: componentName, isComponent: true, props, children: childNodes }];
      }

      return childNodes;
    }

    const tree = processFiber(root.child, 0);
    const allMinified = totalComponents > 0 && minifiedComponents === totalComponents;
    return { found: true, tree, allMinified };
  })()