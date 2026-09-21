/*
 * Prototype-only toolbar for the player study. It is not part of the app
 * build and never ships; it exists so every concept can be judged in all four
 * states the real player must render — playing, preparing (D-038), failed,
 * and the resume banner — plus the idle chrome.
 */
const STATES = [
  ['play', 'Playing'],
  ['prepare', 'Preparing'],
  ['error', 'Failed'],
  ['resume', 'Resume']
];

function mount() {
  const bar = document.createElement('div');
  bar.className = 'studybar';

  const label = document.createElement('span');
  label.textContent = 'State';
  bar.append(label);

  const buttons = STATES.map(([id, text]) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = text;
    button.dataset.state = id;
    button.addEventListener('click', () => {
      document.body.dataset.state = id;
      sync();
    });
    bar.append(button);
    return button;
  });

  const chromeLabel = document.createElement('span');
  chromeLabel.textContent = 'Chrome';
  bar.append(chromeLabel);

  const chrome = document.createElement('button');
  chrome.type = 'button';
  chrome.textContent = 'Toggle';
  chrome.addEventListener('click', () => {
    const hidden = document.body.dataset.chrome === 'hidden';
    document.body.dataset.chrome = hidden ? 'shown' : 'hidden';
    sync();
  });
  bar.append(chrome);

  function sync() {
    for (const button of buttons) {
      button.setAttribute('aria-pressed', String(button.dataset.state === document.body.dataset.state));
    }
    chrome.setAttribute('aria-pressed', String(document.body.dataset.chrome === 'hidden'));
  }

  document.body.append(bar);
  sync();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', mount);
} else {
  mount();
}
