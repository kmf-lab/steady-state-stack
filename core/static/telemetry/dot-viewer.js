const BW = 2; // border width
const BW2 = BW * 2; // border width times 2

const DOT_URL = '/graph.dot';
//const DOT_URL = 'graph.dot';

const ZOOM_DELTA = 40;
//const ZOOM_DELTA = 200;
//const ZOOM_FROM_CENTER = false;
const ZOOM_FROM_CENTER = true;
const ZOOM_MAX = 5000;

const speedMap = {
  'No refresh': 0,
  '40 ms': 40,
  '100 ms': 100,
  '200 ms': 200,
  '1 sec': 1000,
  '10 sec': 10000,
  '20 sec': 20000,
  '1 min': 60000,
  '20 min': 1200000,
  '1 hour': 3600000
};

let aspectRatio, diagram, dragDx, dragDy;
let fitBaseWidth = 0, fitBaseHeight = 0;
let intervalToken, navHeight;
let preview, speedArea, speedDropdown, speedMs, minRefreshRateMs = 0;
let speedSpan, speedText, svg, svgRect, viewport, webworker;
let downloadBtn, downloadDropdown;
let lastDot, lastSvg, lastSuccessAt;
let zoomInBtn, zoomInBtnDisabled, zoomOutBtn, zoomOutBtnDisabled;
let zoomCurrent = 100;

const scroll = (x, y) => window.scrollTo(x, y);

const addClass = (element, name) => (element.className += ' ' + name);

const getById = id => document.querySelector('#' + id);

const hide = element => setStyle(element, 'visibility', 'hidden');

const isVisible = element => element.style.visibility === 'visible';

function setTelemetryTitle(text) {
  const el = getById('telemetryTitle');
  if (el) el.textContent = text;
}

/**
 * Natural SVG size from viewBox, or layout rect after attributes are stripped.
 */
function getSvgNaturalSize(svgEl) {
  const vb = svgEl.viewBox && svgEl.viewBox.baseVal;
  if (vb && vb.width > 0 && vb.height > 0) {
    return {width: vb.width, height: vb.height};
  }
  const rect = svgEl.getBoundingClientRect();
  if (rect.width > 0 && rect.height > 0) {
    return {width: rect.width, height: rect.height};
  }
  return null;
}

/**
 * Triggers a browser download of in-memory text content.
 */
function downloadBlob(text, filename, mime) {
  const url = URL.createObjectURL(new Blob([text], {type: mime}));
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

/**
 * Uniform contain-fit to the viewport, then apply zoomCurrent (preserved on refresh).
 */
function fitDiagramToViewport() {
  if (!svg) return;

  const natural = getSvgNaturalSize(svg);
  if (!natural) return;

  const availW = window.innerWidth;
  const availH = window.innerHeight - navHeight;
  const scale = Math.min(availW / natural.width, availH / natural.height);

  fitBaseWidth = natural.width * scale;
  fitBaseHeight = natural.height * scale;
  aspectRatio = natural.width / natural.height;

  const zoomFactor = zoomCurrent / 100;
  setStyle(diagram, 'width', px(fitBaseWidth * zoomFactor));
  setStyle(diagram, 'height', px(fitBaseHeight * zoomFactor));

  svgRect = svg.getBoundingClientRect();

  const previewRect = preview.getBoundingClientRect();
  setStyle(preview, 'height', px(Math.ceil(previewRect.width / aspectRatio)));

  updateZoomButtons();
  onResize();
}

function updateZoomButtons() {
  if (!fitBaseWidth) return;
  const newWidth = fitBaseWidth * zoomCurrent / 100;
  const canZoomIn = zoomCurrent + ZOOM_DELTA <= ZOOM_MAX;
  const canZoomOut = newWidth > fitBaseWidth + 1;
  setDisplay(zoomInBtn, canZoomIn);
  setDisplay(zoomInBtnDisabled, !canZoomIn);
  setDisplay(zoomOutBtn, canZoomOut);
  setDisplay(zoomOutBtnDisabled, !canZoomOut);
}

function onDownloadDropdown(event) {
  if (downloadBtn.className.indexOf('disabled') !== -1) return;
  toggleVisibility(downloadDropdown);
  if (isVisible(downloadDropdown)) {
    const record = downloadBtn.getBoundingClientRect();
    setStyle(downloadDropdown, 'left', px(record.x + record.width - 180));
  }
  event.stopPropagation();
}

function onDrag(event) {
  const previewRect = preview.getBoundingClientRect();
  const viewportRect = viewport.getBoundingClientRect();
  const newLeft = restrictLeft(
    event.pageX - dragDx - window.scrollX,
    previewRect, viewportRect);
  const newTop = restrictTop(
    event.pageY - dragDy - window.scrollY,
    previewRect, viewportRect);

  // Move viewport.
  setStyle(viewport, 'left', px(newLeft));
  setStyle(viewport, 'top', px(newTop));

  // Move diagram to match location of viewport on preview.
  const xPercent = (newLeft - previewRect.x) / (previewRect.width - BW2);
  const yPercent = (newTop - previewRect.y) / (previewRect.height - BW2);
  const newX = -svgRect.width * xPercent;
  const newY = -svgRect.height * yPercent;
  scroll(-newX, -newY);
}

function onMessage(message) {
  // Live Telemetry only after ok:true + SVG; otherwise Snapshot and keep last diagram.
  const data = message.data;
  // Missing or non-true ok is a failed pull — never default to Live.
  const ok = typeof data === 'object' && data !== null && data.ok === true;
  const svgText = typeof data === 'object' && data !== null ? data.svg : undefined;

  if (!ok || typeof svgText !== 'string') {
    // Pinned to the last successful pull; only a failure before any
    // success falls back to the discovery moment.
    const when = lastSuccessAt || new Date();
    setTelemetryTitle('Snapshot at ' + toLocalIsoWithTz(when));
    return;
  }
  setTelemetryTitle('Live Telemetry');
  lastSuccessAt = new Date();
  lastSvg = svgText;
  if (typeof data.dot === 'string') lastDot = data.dot;
  if (downloadBtn) removeClass(downloadBtn, 'disabled');

  // Use requestAnimationFrame to avoid blocking the UI thread
  window.requestAnimationFrame(() => {
      diagram.innerHTML = svgText;
      removeSvgSize(diagram);

      // Display a scaled copy of the svg in the preview.
      preview.innerHTML = diagram.innerHTML;
      removeSvgSize(preview);

      // Make the preview have the same aspect ratio
      // as the svg that was loaded.
      svg = diagram.querySelector('svg');
      
      // Apply current rendering mode to prevent flicker on refresh
      if (zoomCurrent < 40) {
        if (svg) svg.style.shapeRendering = 'crispEdges';
      } else {
        if (svg) svg.style.shapeRendering = 'geometricPrecision';
      }

      // Copy tooltip <title> elements from each edge group to their
      // label <text> elements so hovering over edge labels shows the
      // same detailed info as hovering over the edge line.
      if (diagram) {
        diagram.querySelectorAll('g[id^="edge"]').forEach(edgeG => {
          const title = edgeG.querySelector(':scope > title');
          if (!title) return;
          const labelText = edgeG.querySelector('text');
          if (labelText && !labelText.querySelector('title')) {
            labelText.prepend(title.cloneNode(true));
          }
        });
      }

      if (svg) {
        fitDiagramToViewport();
      }
  });
}

function onMouseDown(event) {
  // Get distance from mouse down location to upper-left corner
  // of the viewport.  We'll keep this the same throughout the drag.
  const viewportRect = viewport.getBoundingClientRect();
  dragDx = event.pageX - viewportRect.x - window.scrollX;
  dragDy = event.pageY - viewportRect.y - window.scrollY;

  viewport.onmousemove = onDrag;

  viewport.onmouseup = () => {
    // Stop listening for mouse move and mouse up events for now.
    viewport.onmousemove = null;
    viewport.onmouseup = null;
  };
}

/**
 * Resizes viewport to match window.
 */
function onResize() {
  if (!svgRect) return;

  const heightPercent = (window.innerHeight - navHeight) / svgRect.height;
  const widthPercent = window.innerWidth / svgRect.width;

  const previewRect = preview.getBoundingClientRect();
  const {width: pWidth, height: pHeight} = previewRect;

  // Viewport size should represent the ratio of the window to the full diagram
  const vHeight = Math.min(pHeight, pHeight * heightPercent);
  const vWidth = Math.min(pWidth, pWidth * widthPercent);

  setStyle(viewport, 'height', px(vHeight));
  setStyle(viewport, 'width', px(vWidth - BW2));
}

function onScroll() {
  if (!svgRect) return;

  const diagramRect = diagram.getBoundingClientRect();
  const xPercent = -diagramRect.x / svgRect.width;
  const yPercent = (navHeight - diagramRect.y) / svgRect.height;

  const viewportRect = viewport.getBoundingClientRect();
  const {width: vWidth, height: vHeight} = viewportRect;
  const previewRect = preview.getBoundingClientRect();
  const {x: pX, y: pY, width: pWidth, height: pHeight} = previewRect;

  const newX = pX + Math.min(pWidth * xPercent, pWidth - vWidth);
  const newY = pY + Math.min(pHeight * yPercent, pHeight - vHeight);

  setStyle(viewport, 'left', px(newX));
  setStyle(viewport, 'top', px(newY));
}

function onSpeedDropdown(event) {
  toggleVisibility(speedDropdown);
  if (isVisible(speedDropdown)) {
    const record = speedArea.getBoundingClientRect();
    setStyle(speedDropdown, 'left', px(record.x + record.width - 110));
  }
  event.stopPropagation();
}

function onZoom(zoomIn) {
  zoomCurrent += zoomIn ? ZOOM_DELTA : -ZOOM_DELTA;
  console.log("new zoom: " + zoomCurrent);

  // Apply a "Crisp" mode when zoomed out significantly
  if (svg) {
    if (zoomCurrent < 40) {
      svg.style.shapeRendering = 'crispEdges';
    } else {
      svg.style.shapeRendering = 'geometricPrecision';
    }
  }

  let newWidth = fitBaseWidth * zoomCurrent / 100;
  let newHeight = fitBaseHeight * zoomCurrent / 100;
  let newX, newY;

  if (ZOOM_FROM_CENTER) {
    // Move diagram.
    const diagramRect = diagram.getBoundingClientRect();
    const dx = (newWidth - diagramRect.width) / 2;
    const dy = (newHeight - diagramRect.height) / 2;
    newX = diagramRect.left - dx;
    newY = diagramRect.top - dy - navHeight;
  }

  // Must adjust size before attempting to scroll.
  setStyle(diagram, 'width', px(newWidth));
  setStyle(diagram, 'height', px(newHeight));

  if (ZOOM_FROM_CENTER) scroll(-newX, -newY);

  if (svg) svgRect = svg.getBoundingClientRect();
  onResize();
  onScroll();

  updateZoomButtons();
}

const px = text => text + 'px';

function removeClass(element, name) {
  const classes = element.className.split(' ').filter(n => n !== name);
  element.className = classes.join(' ');
}

/**
 * Removes width and height attributes from
 * child svg element so it can be scaled
 * by changing its width.
 */
function removeSvgSize(parent) {
  const svg = parent.querySelector('svg');
  if (svg) {
    svg.removeAttribute('width');
    svg.removeAttribute('height');
  }
}

function restrictLeft(left, previewRect, viewportRect) {
  const {x, width} = previewRect;
  if (left < x) return x;
  const maxX = x + width - viewportRect.width;
  return left > maxX ? maxX : left;
}

function restrictTop(top, previewRect, viewportRect) {
  const {y, height} = previewRect;
  if (top < y) return y;
  const maxY = y + height - viewportRect.height;
  return top > maxY ? maxY : top;
}

const setDisplay = (element, canSee) =>
  setStyle(element, 'display', canSee ? 'block' : 'none');

function enforceMinRefreshRate() {
  Object.keys(speedMap).forEach(key => {
    const ms = speedMap[key];
    const element = document.querySelector('.speed' + ms);
    if (element && ms > 0 && ms < minRefreshRateMs) {
      setStyle(element, 'display', 'none');
      if (speedMs === ms) setSpeed('No refresh');
    } else if (element) {
      setStyle(element, 'display', 'block');
    }
  });
}

function setSpeed(s) {
  const ms = speedMap[s];
  if (ms > 0 && ms < minRefreshRateMs) return;

  // Deselect the currently selected menu item.
  let menuItem = document.querySelector('.speed' + speedMs);
  if (menuItem) removeClass(menuItem, 'selected');

  // Select a new menu item.
  speedText = s;
  speedMs = ms;

  menuItem = document.querySelector('.speed' + speedMs);
  if (menuItem) addClass(menuItem, 'selected');

  speedSpan.textContent = speedText;
  hide(speedDropdown);

  if (intervalToken) clearInterval(intervalToken);
  if (speedMs) {
    intervalToken = setInterval(() => webworker.postMessage(DOT_URL), speedMs);
  }
}

const setStyle = (element, property, value) =>
  (element.style[property] = value);

const show = element => setStyle(element, 'visibility', 'visible');

function togglePreview() {
  toggleVisibility(preview);
  toggleVisibility(viewport);
}

const toggleVisibility = element =>
  isVisible(element) ? hide(element) : show(element);

/**
 * Local time as ISO-8601 with numeric timezone offset,
 * e.g. 2026-08-18T09:40:05-05:00 (toISOString is UTC-only).
 */
function toLocalIsoWithTz(date) {
  const pad = n => String(n).padStart(2, '0');
  const offsetMin = -date.getTimezoneOffset(); // getTimezoneOffset() sign is inverted
  const sign = offsetMin >= 0 ? '+' : '-';
  const abs = Math.abs(offsetMin);
  return date.getFullYear()
    + '-' + pad(date.getMonth() + 1)
    + '-' + pad(date.getDate())
    + 'T' + pad(date.getHours())
    + ':' + pad(date.getMinutes())
    + ':' + pad(date.getSeconds())
    + sign + pad(Math.floor(abs / 60)) + ':' + pad(abs % 60);
}

function fetchConfig() {
  return fetch('/config')
    .then(response => {
        if (!response.ok) throw new Error('Config not found');
        return response.json();
    })
    .then(config => {
      console.log('Config received:', config);
      if (config.telemetry_colors && config.telemetry_colors.length === 2) {
        const primary = config.telemetry_colors[0];
        const secondary = config.telemetry_colors[1];

        const nav1 = getById('nav1');
        const nav2 = getById('nav2');
        if (nav1) nav1.style.backgroundColor = primary;
        if (nav2) {
            nav2.style.backgroundColor = secondary;
            nav2.style.filter = 'none';
        }

        // Inject Dynamic Hover/Selection Styles to override dot-viewer.css
        const styleId = 'steady-dynamic-theme';
        let styleBlock = getById(styleId);
        if (!styleBlock) {
            styleBlock = document.createElement('style');
            styleBlock.id = styleId;
            document.head.appendChild(styleBlock);
        }
        styleBlock.innerHTML = `
            .dropdown > div:hover { background-color: ${primary} !important; }
            .dropdown > .selected { background-color: ${secondary} !important; }
        `;
      }
      if (config.refresh_rate_ms && config.refresh_rate_ms !== minRefreshRateMs) {
        minRefreshRateMs = config.refresh_rate_ms;
        enforceMinRefreshRate();
      }
      return config;
    })
    .catch(err => {
        console.error('Config fetch failed, retrying in 5s:', err);
        setTimeout(fetchConfig, 5000);
    });
}

window.onload = () => {
  if (!window.Worker) {
    alert('Your browser lacks Web Worker support.');
    return;
  }

  diagram = getById('diagram');
  preview = getById('preview');
  speedArea = getById('speedArea');
  speedDropdown = getById('speedDropdown');
  speedSpan = getById('speedSpan');
  speedSpan.textContent = 'Initializing...';
  viewport = getById('viewport');
  downloadBtn = getById('downloadBtn');
  downloadDropdown = getById('downloadDropdown');
  zoomInBtn = getById('zoomInBtn');
  zoomInBtnDisabled = getById('zoomInBtnDisabled');
  zoomOutBtn = getById('zoomOutBtn');
  zoomOutBtnDisabled = getById('zoomOutBtnDisabled');

  getById('previewBtn').onclick = togglePreview;

  const nav2Rect = getById('nav2').getBoundingClientRect();
  navHeight = nav2Rect.y + nav2Rect.height;

  viewport.onmousedown = onMouseDown;

  getById('speedArea').onclick = onSpeedDropdown;

  speedDropdown.onclick = event => setSpeed(event.target.textContent);

  downloadBtn.onclick = onDownloadDropdown;

  downloadDropdown.onclick = event => {
    event.stopPropagation();
    hide(downloadDropdown);
    if (event.target.className.indexOf('downloadDot') !== -1) {
      if (lastDot) downloadBlob(lastDot, 'steady-telemetry.dot', 'text/plain');
    } else if (event.target.className.indexOf('downloadSvg') !== -1) {
      if (lastSvg) downloadBlob(lastSvg, 'steady-telemetry.svg', 'image/svg+xml');
    }
  };

  // Hide all dropdowns on a click outside them.
  window.onclick = () => {
    hide(speedDropdown);
    hide(downloadDropdown);
  };

  zoomInBtn.onclick = () => onZoom(true);
  zoomOutBtn.onclick = () => onZoom(false);

  webworker = new Worker('webworker.js');
  webworker.onmessage = onMessage;
  webworker.postMessage(DOT_URL);

  fetchConfig();
  setSpeed('200 ms');

};

window.onresize = () => {
  if (svg) fitDiagramToViewport();
  else onResize();
};
window.onscroll = onScroll;
