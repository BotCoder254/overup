import { Squirrel } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { cn } from '../../../lib/cn';

/**
 * Interactive particle backdrop for the branding panel, adapted from the
 * "SVG Particle" Framer component. The particle source is the brand's
 * Squirrel mark rendered locally to an SVG data URI (no external image,
 * no canvas tainting). Idle particles roam the panel at reduced opacity;
 * on hover they assemble into the mark and repel around the cursor.
 */

const PARTICLE_COUNT = 50;
const PARTICLE_SIZE = 12;
const ROAM_OPACITY = 0.85;
const REPULSION_FORCE = 10;
const REPULSION_RADIUS = 50;
const TRANSITION_MS = 800;
/** Fraction of the panel the contain-fitted mark occupies. */
const MARK_SCALE = 0.6;

const easeInOut = (t: number) => (t < 0.5 ? 2 * t * t : 1 - 2 * (1 - t) * (1 - t));

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  startX: number;
  startY: number;
  repX: number;
  repY: number;
  homeX: number;
  homeY: number;
  a: number;
  inZone: boolean;
  roamTargetX: number;
  roamTargetY: number;
}

type AnimState = 'idle' | 'assembling' | 'active';

function containRect(iW: number, iH: number, cW: number, cH: number) {
  const a = iW / iH;
  const b = cW / cH;
  return a > b
    ? { x: 0, y: Math.round((cH - cW / a) / 2), w: cW, h: Math.round(cW / a) }
    : { x: Math.round((cW - cH * a) / 2), y: 0, w: Math.round(cH * a), h: cH };
}

function shuffle<T>(items: T[]) {
  for (let i = items.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [items[i], items[j]] = [items[j], items[i]];
  }
}

function randomInRect(bx: number, by: number, bw: number, bh: number): [number, number] {
  return [bx + Math.random() * bw, by + Math.random() * bh];
}

interface ParticleFieldProps {
  className?: string;
}

export function ParticleField({ className }: ParticleFieldProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const iconRef = useRef<SVGSVGElement>(null);

  const particlesRef = useRef<Particle[]>([]);
  const dimsRef = useRef({ W: 0, H: 0 });
  const mouseRef = useRef({ x: -99999, y: -99999, active: false });
  const prevMouseRef = useRef({ x: -99999, y: -99999 });
  const mouseSpeedRef = useRef(0);
  // Smoothed cursor used for repulsion: lerps toward the real cursor so a
  // fast swipe carves one continuous channel instead of discrete rings.
  const smoothMouseRef = useRef({ x: -99999, y: -99999 });
  const animStateRef = useRef<AnimState>('idle');
  const animStartRef = useRef(0);
  const animTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const fadeStartRef = useRef(0);
  const fadeFromRef = useRef(ROAM_OPACITY);
  const fadeToRef = useRef(ROAM_OPACITY);
  const rafRef = useRef(0);

  const startAssembling = () => {
    const now = Date.now();
    for (const p of particlesRef.current) {
      p.startX = p.x;
      p.startY = p.y;
    }
    fadeStartRef.current = now;
    fadeFromRef.current = ROAM_OPACITY;
    fadeToRef.current = 1;
    animStartRef.current = now;
    animStateRef.current = 'assembling';
    clearTimeout(animTimerRef.current);
    animTimerRef.current = setTimeout(() => {
      if (animStateRef.current === 'assembling') animStateRef.current = 'active';
    }, TRANSITION_MS);
  };

  const startRoaming = () => {
    const { W, H } = dimsRef.current;
    for (const p of particlesRef.current) {
      const [tx, ty] = randomInRect(0, 0, W, H);
      p.roamTargetX = tx;
      p.roamTargetY = ty;
    }
    fadeStartRef.current = Date.now();
    fadeFromRef.current = 1;
    fadeToRef.current = ROAM_OPACITY;
    clearTimeout(animTimerRef.current);
    // Roam mode deforms directly out of the mark — no positional tween.
    animStateRef.current = 'idle';
  };

  const initParticles = () => {
    const { W, H } = dimsRef.current;
    const canvas = canvasRef.current;
    const icon = iconRef.current;
    if (!W || !H || !canvas || !icon) return;

    clearTimeout(animTimerRef.current);
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(W * dpr);
    canvas.height = Math.round(H * dpr);
    mouseRef.current = { x: -99999, y: -99999, active: false };
    particlesRef.current = [];

    let markup = new XMLSerializer().serializeToString(icon);
    if (!markup.includes('xmlns=')) {
      markup = markup.replace('<svg', '<svg xmlns="http://www.w3.org/2000/svg"');
    }
    const img = new Image();
    img.onload = () => {
      const base = containRect(img.naturalWidth || 512, img.naturalHeight || 512, W, H);
      const w = base.w * MARK_SCALE;
      const h = base.h * MARK_SCALE;
      const rect = { x: (W - w) / 2, y: (H - h) / 2, w, h };

      const off = document.createElement('canvas');
      off.width = W;
      off.height = H;
      const oc = off.getContext('2d');
      if (!oc) return;
      oc.drawImage(img, rect.x, rect.y, rect.w, rect.h);
      let px: Uint8ClampedArray;
      try {
        px = oc.getImageData(0, 0, W, H).data;
      } catch {
        return;
      }

      const gap = Math.max(2, Math.round(150 / PARTICLE_COUNT));
      const src: Array<{ homeX: number; homeY: number; a: number }> = [];
      for (let y = 0; y < H; y += gap) {
        for (let x = 0; x < W; x += gap) {
          const i = (y * W + x) * 4;
          if (px[i + 3] >= 20) src.push({ homeX: x, homeY: y, a: px[i + 3] });
        }
      }
      shuffle(src);

      particlesRef.current = src.map((s) => {
        const [rx, ry] = randomInRect(0, 0, W, H);
        const [tx, ty] = randomInRect(0, 0, W, H);
        return {
          x: rx,
          y: ry,
          vx: (Math.random() - 0.5) * 1.2,
          vy: (Math.random() - 0.5) * 1.2,
          startX: rx,
          startY: ry,
          repX: 0,
          repY: 0,
          homeX: s.homeX,
          homeY: s.homeY,
          a: s.a,
          inZone: false,
          roamTargetX: tx,
          roamTargetY: ty,
        };
      });
      animStateRef.current = 'idle';
      fadeStartRef.current = 0;
    };
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(markup)}`;
  };

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (!rect) return;
      const W = Math.round(rect.width);
      const H = Math.round(rect.height);
      if (!W || !H) return;
      dimsRef.current = { W, H };
      initParticles();
    });
    observer.observe(el);
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;

    let idata: ImageData | null = null;
    let bW = 0;
    let bH = 0;

    const draw = () => {
      rafRef.current = requestAnimationFrame(draw);
      const PW = canvas.width;
      const PH = canvas.height;
      const particles = particlesRef.current;
      if (!PW || !PH || !particles.length) return;
      const dpr = window.devicePixelRatio || 1;

      if (!idata || PW !== bW || PH !== bH) {
        idata = ctx.createImageData(PW, PH);
        bW = PW;
        bH = PH;
      }
      idata.data.fill(0);
      const buf = idata.data;

      const state = animStateRef.current;
      const { x: rawMx, y: rawMy, active } = mouseRef.current;
      // Capture fresh speed before decay: the impulse must use the raw
      // value, not one already faded by previous frames.
      const hitSpeed = mouseSpeedRef.current;
      mouseSpeedRef.current *= 0.88;

      const sm = smoothMouseRef.current;
      if (active) {
        const lerpFactor = Math.max(0.08, 0.3 - hitSpeed * 0.006);
        if (sm.x < -9000) {
          sm.x = rawMx;
          sm.y = rawMy;
        } else {
          sm.x += (rawMx - sm.x) * lerpFactor;
          sm.y += (rawMy - sm.y) * lerpFactor;
        }
      } else {
        sm.x = -99999;
        sm.y = -99999;
      }

      const ps = Math.max(1, Math.ceil((PARTICLE_SIZE / 4) * dpr));
      const half = ps / 2;
      const elapsed = Date.now() - animStartRef.current;
      const animT = easeInOut(Math.min(1, elapsed / TRANSITION_MS));
      const { W, H } = dimsRef.current;

      let alphaMul: number;
      if (state === 'active') {
        alphaMul = 1;
      } else if (fadeStartRef.current === 0) {
        alphaMul = ROAM_OPACITY;
      } else {
        const fadeT = Math.min(1, Math.max(0, (Date.now() - fadeStartRef.current) / TRANSITION_MS));
        alphaMul = fadeFromRef.current + (fadeToRef.current - fadeFromRef.current) * easeInOut(fadeT);
      }

      const repCutoff = Math.max(1, REPULSION_RADIUS);
      const repCutoffSq = repCutoff * repCutoff;

      const drawParticle = (cx: number, cy: number, alpha: number) => {
        const px0 = Math.round(cx) - (ps >> 1);
        const py0 = Math.round(cy) - (ps >> 1);
        for (let dy = 0; dy < ps; dy++) {
          const iy = py0 + dy;
          if (iy < 0 || iy >= PH) continue;
          const row = iy * PW;
          for (let dx = 0; dx < ps; dx++) {
            const ddx = dx - half + 0.5;
            const ddy = dy - half + 0.5;
            if (ddx * ddx + ddy * ddy > half * half) continue;
            const ix = px0 + dx;
            if (ix < 0 || ix >= PW) continue;
            const i = (row + ix) * 4;
            buf[i] = 0;
            buf[i + 1] = 0;
            buf[i + 2] = 0;
            buf[i + 3] = alpha;
          }
        }
      };

      for (const p of particles) {
        let baseX = p.x;
        let baseY = p.y;
        if (state === 'assembling') {
          baseX = p.startX + (p.homeX - p.startX) * animT;
          baseY = p.startY + (p.homeY - p.startY) * animT;
        } else if (state === 'active') {
          baseX = p.homeX;
          baseY = p.homeY;
        } else {
          // Roaming: drift toward the target, retarget when reached.
          const dtx = p.roamTargetX - p.x;
          const dty = p.roamTargetY - p.y;
          if (Math.sqrt(dtx * dtx + dty * dty) < 3) {
            const [tx, ty] = randomInRect(0, 0, W, H);
            p.roamTargetX = tx;
            p.roamTargetY = ty;
          }
          p.vx = p.vx * 0.98 + (p.roamTargetX - p.x) * 0.003;
          p.vy = p.vy * 0.98 + (p.roamTargetY - p.y) * 0.003;
          const speed = Math.sqrt(p.vx * p.vx + p.vy * p.vy);
          if (speed > 1.5) {
            p.vx = (p.vx / speed) * 1.5;
            p.vy = (p.vy / speed) * 1.5;
          }
          p.x += p.vx;
          p.y += p.vy;
          baseX = p.x;
          baseY = p.y;
        }

        if (active) {
          const dx = baseX - sm.x;
          const dy = baseY - sm.y;
          const distSq = dx * dx + dy * dy;
          if (distSq > 0 && distSq < repCutoffSq) {
            const dist = Math.sqrt(distSq);
            const nx = dx / dist;
            const ny = dy / dist;
            const falloff = 1 - dist / repCutoff;
            const push = falloff * hitSpeed * REPULSION_FORCE * 0.05;
            p.repX += nx * push;
            p.repY += ny * push;
            p.repX += (nx * (repCutoff - dist) - p.repX) * 0.06;
            p.repY += (ny * (repCutoff - dist) - p.repY) * 0.06;
            p.inZone = true;
          } else {
            p.inZone = false;
          }
        } else {
          p.inZone = false;
        }
        if (!p.inZone) {
          p.repX *= 0.97;
          p.repY *= 0.97;
        }
        p.x = baseX + p.repX;
        p.y = baseY + p.repY;

        const alpha = Math.round(p.a * alphaMul);
        if (alpha < 1) continue;
        drawParticle(p.x * dpr, p.y * dpr, alpha);
      }

      ctx.putImageData(idata, 0, 0);
    };

    draw();
    return () => cancelAnimationFrame(rafRef.current);
  }, []);

  const onMouseMove = (event: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    // Normalize from CSS pixels into the intrinsic coordinate space so the
    // repulsion radius stays correct regardless of any transform/zoom.
    const { W, H } = dimsRef.current;
    const scaleX = rect.width > 0 ? W / rect.width : 1;
    const scaleY = rect.height > 0 ? H / rect.height : 1;
    const mx = (event.clientX - rect.left) * scaleX;
    const my = (event.clientY - rect.top) * scaleY;
    const prev = prevMouseRef.current;
    if (prev.x > -9999) {
      const ddx = mx - prev.x;
      const ddy = my - prev.y;
      mouseSpeedRef.current = Math.sqrt(ddx * ddx + ddy * ddy);
    }
    prevMouseRef.current = { x: mx, y: my };
    mouseRef.current = { x: mx, y: my, active: true };
    if (animStateRef.current === 'idle') startAssembling();
  };

  const onMouseLeave = () => {
    mouseRef.current = { x: -99999, y: -99999, active: false };
    prevMouseRef.current = { x: -99999, y: -99999 };
    if (animStateRef.current !== 'idle') startRoaming();
  };

  return (
    <div ref={containerRef} className={cn('overflow-hidden', className)} aria-hidden="true">
      <canvas
        ref={canvasRef}
        className="block h-full w-full"
        onMouseMove={onMouseMove}
        onMouseLeave={onMouseLeave}
      />
      {/* Hidden particle source: serialized to an SVG data URI and sampled. */}
      <div className="absolute h-0 w-0 overflow-hidden" aria-hidden="true">
        <Squirrel ref={iconRef} size={512} strokeWidth={1.25} color="#ffffff" />
      </div>
    </div>
  );
}
