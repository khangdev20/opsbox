// opsbox landing page: theme, copy buttons, mobile menu, reveal on scroll, live preview numbers.
(() => {
  const root = document.documentElement;

  // Theme: saved choice, else system preference.
  const saved = (() => { try { return localStorage.getItem('opsbox.theme'); } catch { return null; } })();
  const prefersLight = window.matchMedia('(prefers-color-scheme: light)').matches;
  const setTheme = (t) => {
    root.dataset.theme = t;
    const btn = document.getElementById('theme');
    if (btn) {
      const label = t === 'light' ? 'Switch to dark theme' : 'Switch to light theme';
      btn.setAttribute('aria-label', label);
      btn.title = label;
      btn.querySelector('.sun').hidden = t === 'light';
      btn.querySelector('.moon').hidden = t !== 'light';
    }
  };
  setTheme(saved || (prefersLight ? 'light' : 'dark'));
  document.getElementById('theme')?.addEventListener('click', () => {
    const next = root.dataset.theme === 'light' ? 'dark' : 'light';
    setTheme(next);
    try { localStorage.setItem('opsbox.theme', next); } catch {}
  });

  // Copy install command.
  document.querySelectorAll('[data-copy]').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const text = document.querySelector(btn.dataset.copy)?.textContent.trim() || '';
      const label = btn.querySelector('span');
      try {
        await navigator.clipboard.writeText(text);
        if (label) label.textContent = 'Copied';
      } catch {
        const r = document.createRange();
        r.selectNodeContents(document.querySelector(btn.dataset.copy));
        const s = getSelection(); s.removeAllRanges(); s.addRange(r);
        if (label) label.textContent = 'Press ⌘C';
      }
      setTimeout(() => { if (label) label.textContent = 'Copy'; }, 1800);
    });
  });

  // Mobile menu.
  const menuBtn = document.getElementById('menu');
  const links = document.getElementById('links');
  menuBtn?.addEventListener('click', () => {
    const open = links.classList.toggle('open');
    menuBtn.setAttribute('aria-expanded', String(open));
  });
  links?.addEventListener('click', (e) => {
    if (e.target.closest('a')) { links.classList.remove('open'); menuBtn?.setAttribute('aria-expanded', 'false'); }
  });

  // Reveal sections as they scroll into view.
  const io = 'IntersectionObserver' in window ? new IntersectionObserver((entries) => {
    entries.forEach((e) => { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } });
  }, { rootMargin: '0px 0px -10% 0px' }) : null;
  document.querySelectorAll('.reveal').forEach((el) => (io ? io.observe(el) : el.classList.add('in')));

  // Make the preview feel alive: gently wander the CPU and network numbers.
  const cpu = document.getElementById('kpi-cpu'), cpuBar = document.getElementById('bar-cpu');
  const net = document.getElementById('kpi-net'), netBar = document.getElementById('bar-net');
  if (cpu && !window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    let c = 25, n = 4.2;
    setInterval(() => {
      c = Math.max(8, Math.min(62, c + (Math.random() - 0.5) * 9));
      n = Math.max(0.6, Math.min(9, n + (Math.random() - 0.5) * 1.4));
      cpu.textContent = Math.round(c);
      cpuBar.style.width = `${c}%`;
      net.textContent = n.toFixed(1);
      netBar.style.width = `${n * 9}%`;
    }, 2000);
  }
})();

// ---- feature demos ----
(() => {
  const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // Deploy: steps run one by one, then live, then restart.
  const steps = [...document.querySelectorAll('#deploy-steps li')];
  const state = document.getElementById('deploy-state');
  if (steps.length) {
    let i = 0;
    const setState = (cls, text) => { state.className = `st ${cls}`; state.textContent = text; };
    const tick = () => {
      if (i === 0) { steps.forEach((s) => (s.className = '')); setState('warn', 'deploying'); }
      if (i > 0) steps[i - 1].className = 'done';
      if (i < steps.length) { steps[i].className = 'run'; i++; }
      else { setState('ok', 'live'); i = 0; return setTimeout(tick, 2600); }
      setTimeout(tick, 900);
    };
    if (still) { steps.forEach((s) => (s.className = 'done')); setState('ok', 'live'); } else tick();
  }

  // Databases: creating -> running, chips appear.
  const dbState = document.getElementById('db-state');
  const chips = document.querySelector('.d-chips');
  if (dbState && !still) {
    const cycle = () => {
      dbState.className = 'st warn'; dbState.textContent = 'creating'; chips.classList.add('hide');
      setTimeout(() => { dbState.className = 'st ok'; dbState.textContent = 'running'; chips.classList.remove('hide'); }, 1800);
    };
    cycle(); setInterval(cycle, 5200);
  } else if (dbState) { dbState.className = 'st ok'; dbState.textContent = 'running'; }

  // Monitoring: rolling CPU line.
  const line = document.getElementById('mon-line'), cpu = document.getElementById('mon-cpu');
  if (line) {
    const pts = Array.from({ length: 31 }, (_, k) => 30 + Math.sin(k / 3) * 12 + Math.random() * 8);
    const draw = () => {
      line.setAttribute('points', pts.map((v, k) => `${k * 10},${70 - v}`).join(' '));
      cpu.textContent = `${Math.round(pts[pts.length - 1])}%`;
    };
    draw();
    if (!still) setInterval(() => { pts.shift(); const last = pts[pts.length - 1]; pts.push(Math.max(8, Math.min(62, last + (Math.random() - 0.5) * 14))); draw(); }, 700);
  }

  // Terminal: type a short session, loop.
  const term = document.getElementById('term');
  if (term) {
    const P = '<span class="p">deploy@srv-sgn-01</span>:<span class="a">~/app</span>$ ';
    const script = [
      ['cmd', 'docker ps --format "{{.Names}}  {{.Status}}"'],
      ['out', 'web-app      Up 3 days\npostgres-15  Up 3 days\nredis        Up 3 days'],
      ['cmd', 'df -h / | tail -1'],
      ['out', '/dev/nvme0n1p1  120G  98G  22G  82% /'],
      ['cmd', 'opsbox status'],
      ['out', '<span class="p">●</span> serve  <span class="p">●</span> priv  <span class="p">●</span> proxy  <span class="p">●</span> watchdog'],
    ];
    if (still) {
      term.innerHTML = script.map(([k, t]) => (k === 'cmd' ? P + t : t)).join('\n');
    } else {
      let html = '', si = 0, ci = 0;
      const step = () => {
        if (si >= script.length) { setTimeout(() => { html = ''; si = 0; ci = 0; step(); }, 2600); return; }
        const [k, t] = script[si];
        if (k === 'out') { html += t + '\n'; si++; term.innerHTML = html + P + '<span class="cur"></span>'; term.scrollTop = term.scrollHeight; return setTimeout(step, 500); }
        if (ci === 0) html += P;
        if (ci < t.length) { html += t[ci].replace('<', '&lt;'); ci++; term.innerHTML = html + '<span class="cur"></span>'; term.scrollTop = term.scrollHeight; return setTimeout(step, 28 + Math.random() * 40); }
        html += '\n'; si++; ci = 0; term.innerHTML = html; term.scrollTop = term.scrollHeight; setTimeout(step, 300);
      };
      step();
    }
  }
})();
