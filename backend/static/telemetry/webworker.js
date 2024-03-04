/*global Viz: false */

const worker = this;

worker.importScripts('viz-lite.js');

worker.onmessage = async message => {
  const dotUrl = message.data;
  if (!dotUrl) return;

  try {
    const response = await fetch(dotUrl);
    const dot = await response.text();
    const svg = Viz(dot, {format: 'svg', engine: 'dot'});
    worker.postMessage(svg);
  } catch (e) {
    console.log('dotUrl',dotUrl);
    console.error('Error:', e);
  }
};
