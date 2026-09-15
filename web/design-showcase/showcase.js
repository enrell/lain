const files = [
  '01-nocturne.html', '02-omarchy-grid.html', '03-archive-suisse.html',
  '04-monolith.html', '05-orbit.html', '06-wired-broadcast.html',
  '07-quiet-luxe.html', '08-kinetic-type.html', '09-contact-sheet.html',
  '10-ambient-room.html'
];

const current = files.findIndex((file) => location.pathname.endsWith(file));
if (current >= 0) {
  const dock = document.createElement('nav');
  dock.className = 'showcase-dock';
  dock.setAttribute('aria-label', 'Design showcase');
  const previous = files[(current + files.length - 1) % files.length];
  const next = files[(current + 1) % files.length];
  dock.innerHTML = `<a href="${previous}" aria-label="Previous concept">←</a><a class="home" href="index.html">${String(current + 1).padStart(2, '0')} / 10</a><a href="${next}" aria-label="Next concept">→</a>`;
  document.body.append(dock);
  addEventListener('keydown', (event) => {
    if (event.key === '[' || event.key === 'ArrowLeft') location.href = previous;
    if (event.key === ']' || event.key === 'ArrowRight') location.href = next;
    if (event.key === 'Escape') location.href = 'index.html';
  });
}

document.querySelectorAll('.sheet-card').forEach((card) => {
  card.addEventListener('click', () => {
    document.querySelectorAll('.sheet-card').forEach((item) => item.classList.remove('selected'));
    card.classList.add('selected');
  });
});
