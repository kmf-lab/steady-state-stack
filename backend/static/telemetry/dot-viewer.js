const BW = 2; // border width
const BW2 = BW * 2; // border width times 2
//const DOT_URL = `http://${host}:${port}/graph.dot`;
//const DOT_URL = 'graph.dot';
//const DOT_URL = 'graph2.dot';
const DOT_URL = 'http://127.0.0.1:8080/graph.dot';
const ZOOM_DELTA = 40;
//const ZOOM_DELTA = 200;
//const ZOOM_FROM_CENTER = false;
const ZOOM_FROM_CENTER = true;
const ZOOM_MAX = 5000;
const MAX_SCALE = 0.4;

const speedMap = {
  'No refresh': 0,
  '33 ms': 33,
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
let preview, speedArea, speedDropdown, speedMs;
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
  const svgText = message.data;
  diagram.innerHTML = svgText;
  removeSvgSize(diagram);

  // Display a scaled copy of the svg in the preview.
  preview.innerHTML = svgText;
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

  // this somewhat fixes it
  setStyle(viewport, 'height', px(vHeight + pHeight * zoomInitialScale));
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
    const rect = speedArea.getBoundingClientRect();
    setStyle(speedDropdown, 'left', px(rect.x + rect.width - 110));
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

function setSpeed(s) {
  // Deselect the currently selected menu item.
  let menuItem = document.querySelector('.speed' + speedMs);
  if (menuItem) removeClass(menuItem, 'selected');

  // Select a new menu item.
  speedText = s;
  speedMs = speedMap[speedText];

  menuItem = document.querySelector('.speed' + speedMs);
  addClass(menuItem, 'selected');

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

  setSpeed('1 sec');

};

window.onresize = onResize;
window.onscroll = onScroll;
