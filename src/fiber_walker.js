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

    function rectFor(el) {
      if (!el || typeof el.getBoundingClientRect !== 'function') return undefined;
      const r = el.getBoundingClientRect();
      const x = Math.round(r.left + (window.scrollX || window.pageXOffset || 0));
      const y = Math.round(r.top + (window.scrollY || window.pageYOffset || 0));
      const width = Math.round(r.width);
      const height = Math.round(r.height);
      if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(width) || !Number.isFinite(height)) return undefined;
      return { x, y, width, height };
    }

    function unionRect(nodes) {
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      let any = false;
      for (const n of nodes || []) {
        const b = n && n.boxRect;
        if (!b) continue;
        any = true;
        minX = Math.min(minX, b.x);
        minY = Math.min(minY, b.y);
        maxX = Math.max(maxX, b.x + b.width);
        maxY = Math.max(maxY, b.y + b.height);
      }
      if (!any) return undefined;
      return { x: minX, y: minY, width: Math.max(0, maxX - minX), height: Math.max(0, maxY - minY) };
    }

    function getComponentName(fiber) {
      if (!fiber.type) return null;
      if (typeof fiber.type === 'string') return null;
      return fiber.type.displayName || fiber.type.name || null;
    }

    function isMinified(name) {
      return name && name.length <= 2 && /^[a-z]/.test(name);
    }

    function summarizeValue(val, depth) {
      depth = depth || 0;
      if (depth > 1) return '{...}';
      if (Array.isArray(val)) {
        if (val.length === 0) return '[]';
        if (val.length <= 3 && val.every(function(v) { return typeof v !== 'object' || v === null; })) {
          return JSON.stringify(val);
        }
        return '[' + val.length + ']';
      }
      if (val && typeof val === 'object') {
        var keys = Object.keys(val);
        if (keys.length === 0) return '{}';
        var parts = [];
        for (var i = 0; i < Math.min(keys.length, 5); i++) {
          var k = keys[i];
          var v = val[k];
          if (typeof v === 'string') parts.push(k + ': "' + (v.length > 30 ? v.slice(0, 30) + '...' : v) + '"');
          else if (typeof v === 'number' || typeof v === 'boolean') parts.push(k + ': ' + v);
          else if (v === null) parts.push(k + ': null');
          else if (Array.isArray(v)) parts.push(k + ': [' + v.length + ']');
          else if (typeof v === 'object') parts.push(k + ': ' + summarizeValue(v, depth + 1));
          else parts.push(k + ': ...');
        }
        var s = '{' + parts.join(', ');
        if (keys.length > 5) s += ', +' + (keys.length - 5);
        return s + '}';
      }
      return String(val);
    }

    function filterProps(props, isHost) {
      if (!props) return {};
      const result = {};
      for (const [key, value] of Object.entries(props)) {
        if (SKIP_PROPS.has(key)) continue;
        if (typeof value === 'function') continue;
        if (isHost) {
          // Host elements: skip events, data attrs, styling
          if (key.startsWith('on')) continue;
          if (key.startsWith('data-sentry-')) continue;
          if (key.startsWith('data-') && !key.startsWith('data-testid') && !key.startsWith('data-gc-')) continue;
          if (STYLING_PROPS.has(key)) continue;
        }
        // React components: show all non-function, non-children props
        if (typeof value === 'string') {
          result[key] = value.length > 80 ? value.slice(0, 80) + '...' : value;
        } else if (typeof value === 'number' || typeof value === 'boolean') result[key] = value;
        else if (value === null) result[key] = null;
        else if (value !== undefined) result[key] = summarizeValue(value);
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
      const domNode = fiber.stateNode && typeof fiber.stateNode.getBoundingClientRect === 'function' ? fiber.stateNode : null;
      const childNodes = processFiber(fiber.child, depth + 1);

      if (isHost && tag) {
        if (!shouldIncludeHost(tag, domNode)) return childNodes;
        const refId = 'e' + (++refCounter);
        if (domNode) domNode.setAttribute('data-ab-ref', refId);
        const ariaInfo = getAriaInfo(domNode);
        const htmlAttrs = getHostAttrs(domNode);
        const props = filterProps(fiber.memoizedProps, true);
        const boxRect = rectFor(domNode);
        return [{ name: tag, isComponent: false, props, ref: refId, boxRect, role: ariaInfo.role, ariaName: ariaInfo.ariaName, tag, htmlAttrs: Object.keys(htmlAttrs).length > 0 ? htmlAttrs : undefined, children: childNodes }];
      }

      if (componentName) {
        totalComponents++;
        if (isMinified(componentName)) { minifiedComponents++; return childNodes; }
        if (SKIP_COMPONENTS.has(componentName)) return childNodes;
        const props = filterProps(fiber.memoizedProps, false);
        const boxRect = unionRect(childNodes);
        return [{ name: componentName, isComponent: true, props, boxRect, children: childNodes }];
      }

      return childNodes;
    }

    const tree = processFiber(root.child, 0);
    const allMinified = totalComponents > 0 && minifiedComponents === totalComponents;
    return { found: true, tree, allMinified };
  })()
