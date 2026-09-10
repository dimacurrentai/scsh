//! Graph viewport controls shared by live jobs and offline snapshots.
//!
//! Browser verification: TEST-OFFLINE-GRAPH.md. Only task navigation is supplied by
//! the page; fitting, zoom, panning, and the modal must work without the daemon.

pub(crate) const WORKFLOW_VIEW_JS: &str = r#"
let workflowZoom = 1;
let workflowExpanded = false;
function wfFitZoom(scroller, stage) {
  if (!scroller || !stage) return 1;
  const naturalWidth = parseFloat(stage.style.width) || stage.scrollWidth || 1;
  const naturalHeight = parseFloat(stage.style.height) || stage.scrollHeight || 1;
  const gutter = 16;
  const widthZoom = Math.max(1, scroller.clientWidth - gutter) / naturalWidth;
  const heightZoom = Math.max(1, scroller.clientHeight - gutter) / naturalHeight;
  // The tighter axis wins, in both directions: a small graph in a large viewport fits
  // UP (>1), a large graph fits down (<1). Callers keep the zoom-out floor at 100%
  // or below, so fitting up never forbids zooming back to natural size.
  return Math.min(widthZoom, heightZoom);
}
function initWorkflowGraphView(activateTask) {
  const root = document.querySelector('[data-workflow-graph]');
  if (!root || root.dataset.bound) return;
  root.dataset.bound = '1';
  const scroller = root.querySelector('.workflow-scroll');
  if (scroller && !scroller.getAttribute('aria-label')) {
    scroller.setAttribute('role', 'region');
    scroller.setAttribute('aria-label', 'Job dependency graph');
    scroller.setAttribute('tabindex', '0');
  }
  const stage = root.querySelector('.workflow-stage');
  const reset = root.querySelector('[data-wf-zoom-reset]');
  const zoomOut = root.querySelector('[data-wf-zoom-out]');
  const expand = root.querySelector('[data-wf-expand]');
  const applyExpanded = (expanded, focusButton) => {
    workflowExpanded = expanded;
    root.classList.toggle('wf-expanded', expanded);
    document.body.classList.toggle('wf-modal-open', expanded);
    if (expanded) {
      root.setAttribute('role', 'dialog');
      root.setAttribute('aria-modal', 'true');
      root.setAttribute('aria-label', 'Job graph, large view');
    } else {
      root.removeAttribute('role');
      root.removeAttribute('aria-modal');
      root.removeAttribute('aria-label');
    }
    if (expand) {
      expand.textContent = expanded ? 'Close' : 'Full screen';
      expand.setAttribute('aria-label', expanded ? 'Close large graph view' : 'Open graph in large view');
      expand.setAttribute('aria-pressed', expanded ? 'true' : 'false');
      if (focusButton) expand.focus({ preventScroll: true });
    }
    // The modal changes both available dimensions. Re-apply the lower bound after layout.
    // The first large-view entry at a given viewport size fits the graph; after that the
    // viewer's chosen zoom wins, keyed per size so a resized window fits fresh once.
    requestAnimationFrame(() => {
      if (expanded && scroller) {
        const key = scroller.clientWidth + 'x' + scroller.clientHeight;
        window.__scshWfExpandFitDone = window.__scshWfExpandFitDone || {};
        if (!window.__scshWfExpandFitDone[key]) {
          window.__scshWfExpandFitDone[key] = true;
          fit();
          return;
        }
      }
      applyZoom(workflowZoom);
    });
  };
  const applyZoom = next => {
    const fit = wfFitZoom(scroller, stage);
    const minimum = Math.min(1, fit);
    // Fit must be able to FILL the viewport as it is right now: when the live fit
    // factor exceeds the manual 2x ceiling (window grown, full screen toggled), the
    // ceiling lifts with it — a fixed cap silently re-fit to the original viewport.
    const maximum = Math.max(2, fit);
    workflowZoom = Math.max(minimum, Math.min(maximum, next));
    // Zoom scales the stage INSIDE the fixed viewport; the card itself never changes size,
    // so zooming (like graph growth) can never reflow the page around it.
    if (stage) stage.style.zoom = String(workflowZoom);
    if (reset) reset.textContent = Math.round(workflowZoom * 100) + '%';
    if (zoomOut) zoomOut.disabled = workflowZoom <= minimum + 0.001;
  };
  const fit = () => {
    if (!stage || !scroller) return;
    applyZoom(wfFitZoom(scroller, stage));
    scroller.scrollLeft = 0;
    scroller.scrollTop = 0;
  };
  // Zoom anchored at a viewport point: the content under the pointer stays put. CSS zoom
  // scales the scroll content linearly, so re-anchoring is pure arithmetic on the offsets.
  const zoomAt = (next, clientX, clientY) => {
    if (!scroller) { applyZoom(next); return; }
    const rect = scroller.getBoundingClientRect();
    const px = clientX - rect.left;
    const py = clientY - rect.top;
    const prev = workflowZoom;
    applyZoom(next);
    if (workflowZoom === prev) return;
    const ratio = workflowZoom / prev;
    scroller.scrollLeft = (scroller.scrollLeft + px) * ratio - px;
    scroller.scrollTop = (scroller.scrollTop + py) * ratio - py;
  };
  // Animated variant for discrete gestures (double-click): glide to the target zoom over a
  // few frames, keeping the same anchor point, instead of jumping. Wheel/pinch stays
  // per-event — those gestures are already continuous.
  const animateZoomAt = (target, clientX, clientY) => {
    const reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (reduce) { zoomAt(target, clientX, clientY); return; }
    const from = workflowZoom;
    const start = performance.now();
    const ms = 200;
    const step = now => {
      const t = Math.min(1, (now - start) / ms);
      const eased = 1 - Math.pow(1 - t, 3);
      zoomAt(from + (target - from) * eased, clientX, clientY);
      if (t < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  };
  // Global modal handlers must always act on the current graph node. Live job updates preserve
  // this card when possible, but a late graph mount can still introduce it after page load.
  // Expose the closure on the node instead of capturing a stale element in a document listener.
  root.__scshApplyWorkflowExpanded = applyExpanded;
  // The page opens on the fitted graph. Only the first mount per page load fits: live
  // updates remount the card, and a remount must keep the zoom the viewer chose.
  if (!window.__scshWfInitialFitDone) {
    window.__scshWfInitialFitDone = true;
    fit();
  } else {
    applyZoom(workflowZoom);
  }
  zoomOut?.addEventListener('click', () => applyZoom(workflowZoom - 0.1));
  root.querySelector('[data-wf-zoom-in]')?.addEventListener('click', () => applyZoom(workflowZoom + 0.1));
  reset?.addEventListener('click', () => applyZoom(1));
  root.querySelector('[data-wf-zoom-fit]')?.addEventListener('click', fit);
  expand?.addEventListener('click', () => applyExpanded(!workflowExpanded, false));
  applyExpanded(workflowExpanded, false);
  root.__scshApplyWorkflowZoom = () => applyZoom(workflowZoom);
  if (!window.__scshWfResizeBound) {
    window.__scshWfResizeBound = true;
    window.addEventListener('resize', () => {
      const current = document.querySelector('[data-workflow-graph]');
      if (current && current.__scshApplyWorkflowZoom) current.__scshApplyWorkflowZoom();
    });
  }
  if (!window.__scshWfExpandBound) {
    window.__scshWfExpandBound = true;
    document.addEventListener('click', ev => {
      if (!workflowExpanded) return;
      const current = document.querySelector('[data-workflow-graph]');
      // The body's fixed ::before layer is the visual backdrop. A click landing anywhere outside
      // the inset card expresses dismissal; clicks on the graph, controls, or nodes stay inside.
      if (!current || current.contains(ev.target)) return;
      if (current.__scshApplyWorkflowExpanded) current.__scshApplyWorkflowExpanded(false, false);
    });
    document.addEventListener('keydown', ev => {
      if (!workflowExpanded) return;
      const current = document.querySelector('[data-workflow-graph]');
      if (ev.key === 'Tab' && current) {
        const focusable = Array.from(current.querySelectorAll('a[href],button,[tabindex]:not([tabindex="-1"])'))
          .filter(el => !el.disabled && el.getClientRects().length > 0);
        if (focusable.length) {
          const first = focusable[0], last = focusable[focusable.length - 1];
          if (ev.shiftKey && document.activeElement === first) {
            ev.preventDefault();
            last.focus();
          } else if (!ev.shiftKey && document.activeElement === last) {
            ev.preventDefault();
            first.focus();
          }
        }
        return;
      }
      if (ev.key !== 'Escape') return;
      const button = current && current.querySelector('[data-wf-expand]');
      workflowExpanded = false;
      document.body.classList.remove('wf-modal-open');
      if (current) {
        current.classList.remove('wf-expanded');
        current.removeAttribute('role');
        current.removeAttribute('aria-modal');
        current.removeAttribute('aria-label');
      }
      if (button) {
        button.textContent = 'Full screen';
        button.setAttribute('aria-label', 'Open graph in large view');
        button.setAttribute('aria-pressed', 'false');
        button.focus({ preventScroll: true });
      }
    });
  }
  scroller?.addEventListener('wheel', ev => {
    ev.preventDefault();
    ev.stopPropagation();
    if (ev.ctrlKey || ev.metaKey) {
      // Trackpad pinch arrives as ctrlKey wheel events: zoom toward the fingers, not the
      // viewport center.
      zoomAt(workflowZoom + (ev.deltaY < 0 ? 0.1 : -0.1), ev.clientX, ev.clientY);
      return;
    }
    scroller.scrollLeft += ev.deltaX;
    scroller.scrollTop += ev.deltaY;
  }, { passive: false });
  scroller?.addEventListener('dblclick', ev => {
    // Double-clicking a node or control keeps its own meaning; empty graph area zooms in
    // at the clicked point.
    if (ev.target.closest('a.wf-node, a.wf-jump, button')) return;
    ev.preventDefault();
    animateZoomAt(workflowZoom * 1.5, ev.clientX, ev.clientY);
  });
  // Mouse drag on empty graph area pans the viewport — wheel-less mice and rough touchpads
  // need a way to move around. Touch keeps the container's native scrolling (touch-action);
  // a 3px dead zone leaves double-click jitter alone.
  scroller?.addEventListener('pointerdown', ev => {
    if (ev.pointerType !== 'mouse' || ev.button !== 0) return;
    if (ev.target.closest('a.wf-node, a.wf-jump, button')) return;
    ev.preventDefault();
    const startX = ev.clientX, startY = ev.clientY;
    const startLeft = scroller.scrollLeft, startTop = scroller.scrollTop;
    let panning = false;
    const move = e => {
      const dx = e.clientX - startX, dy = e.clientY - startY;
      if (!panning && Math.abs(dx) + Math.abs(dy) < 3) return;
      panning = true;
      scroller.classList.add('wf-panning');
      scroller.scrollLeft = startLeft - dx;
      scroller.scrollTop = startTop - dy;
    };
    const up = () => {
      scroller.removeEventListener('pointermove', move);
      scroller.removeEventListener('pointerup', up);
      scroller.removeEventListener('pointercancel', up);
      scroller.classList.remove('wf-panning');
    };
    try { scroller.setPointerCapture(ev.pointerId); } catch (_) {}
    scroller.addEventListener('pointermove', move);
    scroller.addEventListener('pointerup', up);
    scroller.addEventListener('pointercancel', up);
  });
  root.addEventListener('click', (ev) => {
    const jump = ev.target.closest('a.wf-jump');
    if (jump && root.contains(jump)) {
      const href = jump.getAttribute('href') || '';
      const m = /^#task-(.+)$/.exec(href);
      if (!m) return;
      ev.preventDefault();
      let step;
      try { step = decodeURIComponent(m[1]); } catch (_) { return; }
      if (workflowExpanded) applyExpanded(false, false);
      activateTask(step);
      return;
    }
    const a = ev.target.closest('a.wf-node');
    if (!a || !root.contains(a)) return;
    const step = a.getAttribute('data-workflow-step');
    if (!step) return;
    ev.preventDefault();
    if (workflowExpanded) applyExpanded(false, false);
    activateTask(step);
  });
}
"#;
