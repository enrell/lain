const files = ['01-shelf.html', '02-focus.html', '03-rows.html'];

const current = files.findIndex((file) => location.pathname.endsWith(file));
if (current >= 0 && window.self === window.top) {
  const dock = document.createElement('nav');
  dock.className = 'showcase-dock';
  dock.setAttribute('aria-label', 'Library concepts');
  const previous = files[(current + files.length - 1) % files.length];
  const next = files[(current + 1) % files.length];
  dock.innerHTML = `<a href="${previous}" aria-label="Previous concept">←</a><a class="home" href="index.html">${String(current + 1).padStart(2, '0')} / 03</a><a href="${next}" aria-label="Next concept">→</a>`;
  document.body.append(dock);
  addEventListener('keydown', (event) => {
    if (event.key === '[' || event.key === 'ArrowLeft') location.href = previous;
    if (event.key === ']' || event.key === 'ArrowRight') location.href = next;
    if (event.key === 'Escape') location.href = 'index.html';
  });
}

const SHOWS = {
  frieren: {
    title: 'Frieren', meta: '2023 · 1 season · 28 episodes',
    desc: 'An elf mage retraces a decade-long journey and learns what a brief human life meant to her.',
    resume: 'Episode 9 · 42% watched'
  },
  silo: {
    title: 'Silo', meta: '2023 · 2 seasons · 20 episodes',
    desc: 'Thousands live deep underground under rules nobody is allowed to question.',
    resume: 'Season 2 · Episode 1'
  },
  darling: {
    title: 'Darling in the FranXX', meta: '2018 · 1 season · 24 episodes',
    desc: 'Young pilots search for identity and connection while defending the last remnants of civilization.',
    resume: 'Episode 12 · 50% watched'
  },
  piece: {
    title: 'One Piece', meta: '1999 · 21 seasons · 1100+ episodes',
    desc: 'A pirate crew crosses impossible seas in pursuit of freedom and a legendary treasure.',
    resume: 'Episode 1043'
  },
  lucky: {
    title: 'Lucky', meta: '2025 · 1 season · 7 episodes',
    desc: 'A quiet town, a strange lottery, and seven episodes to understand what luck costs.',
    resume: 'Not started'
  },
  witch: {
    title: 'Witch Hat Atelier', meta: '2025 · 1 season · 12 episodes',
    desc: 'A girl who longs for magic discovers that wonder and discipline grow together.',
    resume: 'Episode 2'
  }
};

function selectShow(key) {
  const show = SHOWS[key];
  if (!show) return;
  document.querySelectorAll('[data-show]').forEach((el) => {
    el.classList.toggle('selected', el.dataset.show === key);
  });
  document.querySelectorAll('[data-detail-title]').forEach((el) => { el.textContent = show.title; });
  document.querySelectorAll('[data-detail-meta]').forEach((el) => { el.textContent = show.meta; });
  document.querySelectorAll('[data-detail-desc]').forEach((el) => { el.textContent = show.desc; });
  document.querySelectorAll('[data-detail-resume]').forEach((el) => { el.textContent = show.resume; });
  const head = document.querySelector('[data-detail-head]');
  if (head) head.className = `detail-head bg-${key}`;
}

document.querySelectorAll('[data-show]').forEach((card) => {
  card.addEventListener('click', () => {
    selectShow(card.dataset.show);
    document.querySelectorAll('.drawer, .scrim').forEach((el) => el.classList.add('open'));
    const acc = document.querySelector(`[data-acc="${card.dataset.show}"]`);
    document.querySelectorAll('.episodes').forEach((el) => { el.hidden = true; });
    if (acc) acc.hidden = false;
  });
});

document.querySelectorAll('[data-close]').forEach((btn) => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.drawer, .scrim').forEach((el) => el.classList.remove('open'));
  });
});

document.querySelectorAll('[data-seasons]').forEach((group) => {
  group.querySelectorAll('[data-season]').forEach((btn) => {
    btn.addEventListener('click', () => {
      group.querySelectorAll('[data-season]').forEach((el) => el.classList.remove('active'));
      btn.classList.add('active');
      const season = btn.dataset.season || 'all';
      document.querySelectorAll('[data-epseason]').forEach((ep) => {
        ep.hidden = season !== 'all' && ep.dataset.epseason !== season;
      });
    });
  });
});
document.querySelectorAll('[data-lib]').forEach((btn) => {
  btn.addEventListener('click', () => {
    document.querySelector(`[data-libgroup]`)?.querySelectorAll('[data-lib]').forEach((el) => el.classList.remove('active'));
    btn.classList.add('active');
    const lib = btn.dataset.lib;
    document.querySelectorAll('[data-libs]').forEach((el) => {
      el.hidden = lib !== 'all' && !el.dataset.libs.split(' ').includes(lib);
    });
  });
});

const searchInput = document.querySelector('[data-search]');
if (searchInput) {
  searchInput.addEventListener('input', () => {
    const q = searchInput.value.trim().toLowerCase();
    document.querySelectorAll('[data-title]').forEach((el) => {
      const owner = el.closest('[data-libs]') || el;
      const hit = el.dataset.title.toLowerCase().includes(q);
      if (owner !== el) owner.hidden = !hit && q.length > 0 ? true : owner.hidden && q.length > 0 ? true : false;
      if (owner === el) owner.hidden = !hit;
    });
  });
}

document.querySelectorAll('[data-sort]').forEach((sel) => {
  sel.addEventListener('change', () => {
    document.querySelectorAll('[data-sortable]').forEach((grid) => {
      const cards = [...grid.querySelectorAll(':scope > [data-libs]')];
      cards.sort((a, b) => {
        if (sel.value === 'recent') return (b.dataset.recent || '').localeCompare(a.dataset.recent || '');
        return (a.dataset.title || '').localeCompare(b.dataset.title || '');
      });
      cards.forEach((c) => grid.append(c));
    });
  });
});
