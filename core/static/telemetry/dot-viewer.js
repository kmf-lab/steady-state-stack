const BW = 2; // border width
const BW2 = BW * 2; // border width times 2

const {host} = this.location;
const DOT_URL = `http://${host}/graph.dot`;
//const DOT_URL = 'graph.dot';

const ZOOM_DELTA = 40;
//const ZOOM_DELTA = 200;
//const ZOOM_FROM_CENTER = false;
const ZOOM_FROM_CENTER = true;
const ZOOM_MAX = 5000;
const MAX_SCALE = 0.4;

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

let aspectRatio, diagram, dragDx, dragDy, exportAnchor;
let firstTime = true;
let intervalToken, navHeight, originalWindowWidth;
let preview, speedArea, speedDropdown, speedMs, minRefreshRateMs = 0;
let speedSpan, speedText, svg, svgRect, userDropdown, viewport, webworker;
let zoomInBtn, zoomInBtnDisabled, zoomOutBtn, zoomOutBtnDisabled;
let zoomCurrent = 100, zoomInitialScale = 1.0;

const addClass = (element, name) => (element.className += ' ' + name);

const getById = id => document.querySelector('#' + id);

function getFileName() {
  const date = new Date();
  const year = date.getFullYear().toString();
  const month = (date.getMonth() + 1).toString();
  const day = date.getDate().toString();
  const hour = date.getHours().toString();
  const min = date.getMinutes().toString();
  const sec = date.getSeconds().toString();
  return `telemetry_${year}-${month}-${day}_${hour}-${min}-${sec}.svg`;
}

const hide = element => setStyle(element, 'visibility', 'hidden');

const isVisible = element => element.style.visibility === 'visible';

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

function onExport() {
  var data = diagram.innerHTML;

  // Invert the graph.
  data = data.replace(/stroke="#ffffff"/g, 'stroke="#000000"')
      .replace(/stroke="#b2b2b2"/g, 'stroke="#4d4d4d"')
      .replace(/fill="#ffffff"/g, 'fill="#000000"')
      .replace('polygon fill="#000000"', 'polygon fill="#ffffff"');

  const blob = new Blob([data], {type: 'octet/stream'});
  const url = window.URL.createObjectURL(blob);
  exportAnchor.download = getFileName();
  exportAnchor.href = url;
  exportAnchor.click();
  window.URL.revokeObjectURL(url);
}

function onMessage(message) {
  let svgText;
  if (typeof message.data === 'string') {
    svgText = message.data;
  } else {
    ({ svg: svgText } = message.data);
  }

  diagram.innerHTML = svgText;
  removeSvgSize(diagram);

  // Display a scaled copy of the svg in the preview.
  preview.innerHTML = diagram.innerHTML;
  removeSvgSize(preview);

  // Make the preview have the same aspect ratio
  // as the svg that was loaded.
  svg = diagram.querySelector('svg');
  svgRect = svg.getBoundingClientRect();
  aspectRatio = svgRect.width / svgRect.height;
  const previewRect = preview.getBoundingClientRect();
  const newHeight = Math.ceil(previewRect.width / aspectRatio);
  setStyle(preview, 'height', px(newHeight));

  if (firstTime) {
    firstTime = false;

    const diagramRect = diagram.getBoundingClientRect();

    // if diagram is taller than the window, then we zoom out
    // if it is wider, ignore and just set it to original width
    if(diagramRect.height > window.innerHeight) {
       zoomInitialScale = MAX_SCALE;
       originalWindowWidth *= zoomInitialScale;
    }

    setStyle(diagram, 'width', px(originalWindowWidth));

    // Make the viewport start at the same size as the preview.
    onResize();
  }
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
  const {x: pX, y: pY, width: pWidth, height: pHeight} = previewRect;
  const viewportRect = viewport.getBoundingClientRect();
  const {x: vX, y: vY} = viewportRect;

  // Resize viewport to match.

  let vHeight = Math.min(pY + pHeight - vY, pHeight * heightPercent);
  let vWidth = Math.min(pX + pWidth - vX, pWidth * widthPercent);

  const pRight = pX + pWidth;
  if (vX + vWidth > pRight) {
    vWidth = pRight - vX;
    setStyle(viewport, 'x', px(pRight - vWidth));
  }

  const pBottom = pY + pHeight;
  if (vY + vHeight > pBottom) {
    vHeight = pBottom - vY;
    setStyle(viewport, 'y', px(pBottom - vHeight));
  }

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
  hide(userDropdown);
  toggleVisibility(speedDropdown);
  if (isVisible(speedDropdown)) {
    const record = speedArea.getBoundingClientRect();
    setStyle(speedDropdown, 'left', px(record.x + record.width - 110));
  }
  event.stopPropagation();
}

function onUser(event) {
  hide(speedDropdown);
  toggleVisibility(userDropdown);
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

  let newWidth = originalWindowWidth * zoomCurrent / 100;
  let newHeight = newWidth / aspectRatio;
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

  // Determine which zoom buttons should be displayed.
  const canZoomIn = zoomCurrent + ZOOM_DELTA <= ZOOM_MAX;
  const canZoomOut = newWidth > window.innerWidth * zoomInitialScale + 1;
  setDisplay(zoomInBtn, canZoomIn);
  setDisplay(zoomInBtnDisabled, !canZoomIn);
  setDisplay(zoomOutBtn, canZoomOut);
  setDisplay(zoomOutBtnDisabled, !canZoomOut);

  // If diagram is larger than browser window,
  // change viewport size to represent visible area.
  const {innerHeight, innerWidth} = window;
  svgRect = svg.getBoundingClientRect();
  const widthPercent = Math.min(1, innerWidth / svgRect.width);
  const heightPercent = Math.min(1, (innerHeight - navHeight) / svgRect.height);
  const previewRect = preview.getBoundingClientRect();
  newWidth = (previewRect.width - BW2) * widthPercent;
  newHeight = (previewRect.height - BW2) * heightPercent;

  setStyle(viewport, 'height', px(newHeight));
  setStyle(viewport, 'width', px(newWidth));

  if (ZOOM_FROM_CENTER) {
    // Move viewport.
    const dx = (previewRect.width - newWidth) / 2;
    const dy = (previewRect.height - newHeight) / 2;
    newX = previewRect.left + dx - BW;
    newY = previewRect.top + dy - BW;
    setStyle(viewport, 'left', px(newX));
    setStyle(viewport, 'top', px(newY));
  }
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
  svg.removeAttribute('width');
  svg.removeAttribute('height');
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

function fetchConfig() {
  return fetch('/config')
    .then(response => response.json())
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
    .catch(err => console.error('Config fetch failed:', err));
}

window.onload = () => {
  if (!window.Worker) {
    alert('Your browser lacks Web Worker support.');
    return;
  }

  originalWindowWidth = window.innerWidth;

  exportAnchor = document.createElement('a');

  diagram = getById('diagram');
  preview = getById('preview');
  speedArea = getById('speedArea');
  speedDropdown = getById('speedDropdown');
  speedSpan = getById('speedSpan');
  userDropdown = getById('userDropdown');
  viewport = getById('viewport');
  zoomInBtn = getById('zoomInBtn');
  zoomInBtnDisabled = getById('zoomInBtnDisabled');
  zoomOutBtn = getById('zoomOutBtn');
  zoomOutBtnDisabled = getById('zoomOutBtnDisabled');

  getById('previewBtn').onclick = togglePreview;

  const nav2Rect = getById('nav2').getBoundingClientRect();
  navHeight = nav2Rect.y + nav2Rect.height;

  viewport.onmousedown = onMouseDown;

  getById('speedArea').onclick = onSpeedDropdown;
  getById('userBtn').onclick = onUser;
  getById('exportItem').onclick = onExport;

  speedDropdown.onclick = event => setSpeed(event.target.textContent);
  userDropdown.onclick = () => hide(userDropdown);

  // Hide all dropdowns on a click outside them.
  window.onclick = () => {
    hide(speedDropdown);
    hide(userDropdown);
  };

  zoomInBtn.onclick = () => onZoom(true);
  zoomOutBtn.onclick = () => onZoom(false);

  webworker = new Worker('webworker.js');
  webworker.onmessage = onMessage;
  webworker.postMessage(DOT_URL);

  fetchConfig().then(() => {
    setSpeed('200 ms');
  });

};

window.onresize = onResize;
window.onscroll = onScroll;
