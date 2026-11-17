<script lang="ts"></script>

<!--
  Three documents go into a cyan trash can: the picture of a history with no cleanup in it yet. The lid swings up on
  its right-hand hinge until it stands clear of the opening, the documents follow one another over the can and drop
  straight down through the middle of it, and each one vanishes below the rim (the clip hides what is inside the can).
  Then the lid shuts with a small bounce and the can gives a little. 5.2 s a loop. Still under reduced motion
  (app.css): three documents beside a closed can.
-->
<svg class="art" width="140" height="72" viewBox="0 0 140 72" fill="none" aria-hidden="true">
  <defs>
    <!-- Everything but the inside of the can: above the rim, and left and right of its walls. -->
    <clipPath id="history-outside-can" clipPathUnits="userSpaceOnUse">
      <path d="M-40 -60 H180 V30 H-40 Z M-40 30 H97 V90 H-40 Z M125 30 H180 V90 H125 Z" />
    </clipPath>
    <!-- Smoke: soft at the edge without a blur filter (the app window does not blur SVG with CSS filters). -->
    <radialGradient id="history-smoke">
      <stop offset="0" stop-color="var(--muted)" stop-opacity="0.75" />
      <stop offset="0.55" stop-color="var(--muted)" stop-opacity="0.35" />
      <stop offset="1" stop-color="var(--muted)" stop-opacity="0" />
    </radialGradient>
  </defs>

  <g clip-path="url(#history-outside-can)">
    <g class="doc d1">
      <rect x="0" y="34" width="20" height="26" rx="3" fill="var(--amber)" />
      <path d="M4 42h12M4 48h9" stroke="var(--ink-1)" stroke-width="2" stroke-linecap="round" />
    </g>
    <g class="doc d2">
      <g transform="rotate(-8 36 39)">
        <rect x="26" y="26" width="20" height="26" rx="3" fill="var(--slate)" />
        <path d="M30 34h12M30 40h9" stroke="var(--ink-1)" stroke-width="2" stroke-linecap="round" />
      </g>
    </g>
    <g class="doc d3">
      <g transform="rotate(6 62 49)">
        <rect x="52" y="36" width="20" height="26" rx="3" fill="var(--violet)" />
        <path d="M56 44h12M56 50h9" stroke="var(--ink-1)" stroke-width="2" stroke-linecap="round" />
      </g>
    </g>
  </g>
  <g class="bump">
    <path
      d="M96 30 L99 66 H123 L126 30 Z"
      fill="var(--cyan)"
      stroke="var(--cyan)"
      stroke-width="2.5"
      stroke-linejoin="round"
    />
    <path
      d="M104 38v20M111 38v20M118 38v20"
      stroke="var(--ink-2)"
      stroke-width="2.4"
      stroke-linecap="round"
    />
  </g>
  <!-- The gust when the lid slams: puffs of air out of both sides of the lid, and wind lines sweeping away. -->
  <g class="gust" aria-hidden="true">
    <circle class="puff pl1" cx="93" cy="27" r="5" />
    <circle class="puff pl2" cx="96" cy="31" r="4" />
    <circle class="puff pl3" cx="95" cy="23" r="3.5" />
    <circle class="puff pr1" cx="128" cy="27" r="5" />
    <circle class="puff pr2" cx="125" cy="31" r="4" />
    <circle class="puff pr3" cx="126" cy="23" r="3.5" />
    <path class="wind wl1" d="M88 27 c-5 0 -10 -1 -14 -4" pathLength="10" />
    <path class="wind wl2" d="M90 31 c-4 1 -8 1 -11 -1" pathLength="10" />
    <path class="wind wr1" d="M133 27 c5 0 10 -1 14 -4" pathLength="10" />
    <path class="wind wr2" d="M131 31 c4 1 8 1 11 -1" pathLength="10" />
  </g>
  <g class="lid">
    <path d="M92 25h37" stroke="var(--cyan)" stroke-width="4" stroke-linecap="round" />
    <path d="M105 23v-4h12v4" stroke="var(--cyan)" stroke-width="2.5" stroke-linejoin="round" />
  </g>
</svg>

<style>
  .art {
    flex: none;
    display: block;
    overflow: visible;
  }

  .doc {
    transform-box: fill-box;
    transform-origin: center;
    animation: fly 5.2s linear infinite;
  }

  /* From each document's centre to the point above the opening (x 111) and down inside the can. */
  .d1 {
    --tx: 101px;
    --ay: -41px;
    --ey: 14px;
  }

  .d2 {
    --tx: 75px;
    --ay: -33px;
    --ey: 22px;
    animation-delay: 0.45s;
  }

  .d3 {
    --tx: 49px;
    --ay: -43px;
    --ey: 12px;
    animation-delay: 0.9s;
  }

  @keyframes fly {
    0%,
    14% {
      transform: translate(0, 0) rotate(0) scale(1);
      opacity: 1;
      animation-timing-function: cubic-bezier(0.3, 0, 0.5, 1);
    }

    /* Up and over: the top of the arc is right above the opening. */
    30% {
      transform: translate(var(--tx), var(--ay)) rotate(-6deg) scale(0.62);
      animation-timing-function: cubic-bezier(0.5, 0, 0.9, 0.5);
    }

    /* Straight down into the can, upright and narrower than the opening. */
    38% {
      transform: translate(var(--tx), var(--ey)) rotate(0deg) scale(0.56);
      opacity: 1;
      animation-timing-function: step-end;
    }

    /* Inside the can it is hidden first and only then put back at the start, so it never travels back out. */
    38.5% {
      transform: translate(var(--tx), var(--ey)) rotate(0deg) scale(0.56);
      opacity: 0;
      animation-timing-function: step-end;
    }

    39%,
    86% {
      transform: translate(0, 0) scale(0.6);
      opacity: 0;
    }

    100% {
      transform: translate(0, 0) scale(1);
      opacity: 1;
    }
  }

  /* The lid stands almost upright on its hinge while the documents fall, clear of the opening. */
  .lid {
    transform-origin: 128px 25px;
    animation: lid 5.2s var(--ease-out) infinite;
  }

  @keyframes lid {
    0%,
    16% {
      transform: rotate(0);
    }

    26%,
    60% {
      transform: rotate(78deg);
    }

    68% {
      transform: rotate(-4deg);
    }

    73%,
    100% {
      transform: rotate(0);
    }
  }

  .bump {
    transform-box: fill-box;
    transform-origin: bottom center;
    animation: bump 5.2s var(--ease-out) infinite;
  }

  @keyframes bump {
    0%,
    64% {
      transform: scale(1);
    }

    69% {
      transform: scale(1.06, 0.94);
    }

    76%,
    100% {
      transform: scale(1);
    }
  }
  /* The gust: starts the moment the lid hits the can (68 %), gone well before the next documents fly. */
  .puff {
    fill: url(#history-smoke);
    opacity: 0;
    transform-box: fill-box;
    transform-origin: center;
    animation: puff 5.2s var(--ease-out) infinite;
  }

  .pl1,
  .pl2,
  .pl3 {
    --dx: -11px;
  }

  .pr1,
  .pr2,
  .pr3 {
    --dx: 11px;
  }

  .pl2,
  .pr2 {
    --dy: 3px;
    animation-delay: 0.05s;
  }

  .pl3,
  .pr3 {
    --dy: -4px;
    animation-delay: 0.1s;
  }

  @keyframes puff {
    0%,
    66% {
      opacity: 0;
      transform: translate(0, 0) scale(0.4);
    }

    69% {
      opacity: 0.9;
      transform: translate(calc(var(--dx) * 0.3), calc(var(--dy, 0px) * 0.3)) scale(1);
    }

    82%,
    100% {
      opacity: 0;
      transform: translate(var(--dx), var(--dy, 0px)) scale(2.4);
    }
  }

  .wind {
    stroke: var(--muted);
    stroke-width: 1.2;
    stroke-linecap: round;
    stroke-dasharray: 10;
    stroke-dashoffset: 10;
    opacity: 0;
    animation: wind 5.2s var(--ease-out) infinite;
  }

  .wl2,
  .wr2 {
    animation-delay: 0.06s;
  }

  @keyframes wind {
    0%,
    67% {
      stroke-dashoffset: 10;
      opacity: 0;
    }

    70% {
      opacity: 0.6;
    }

    76% {
      stroke-dashoffset: 0;
      opacity: 0.5;
    }

    84%,
    100% {
      stroke-dashoffset: -10;
      opacity: 0;
    }
  }
</style>
