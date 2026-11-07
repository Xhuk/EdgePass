/**
 * The app mark: a rounded-square tile on Apple's icon grid with a keyhole in a
 * shield.
 *
 * Two craft fixes over the old one: the baked-in "EDGE" wordmark is gone (at
 * 22px it was an illegible smudge, and the header already says edgePass next
 * to it), and the tile now carries a real specular highlight instead of a flat
 * two-stop gradient.
 */
export function LogoMark({
  className = "",
  size = 24,
}: {
  className?: string;
  size?: number;
}) {
  return (
    <span className={`logo-mark ${className}`.trim()} style={{ width: size, height: size }} aria-hidden="true">
      <svg viewBox="0 0 28 28" fill="none" xmlns="http://www.w3.org/2000/svg">
        <defs>
          <linearGradient id="ep-tile" x1="5" y1="1" x2="23" y2="27" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#66cbbc" />
            <stop offset="100%" stopColor="#1d665f" />
          </linearGradient>
          <linearGradient id="ep-gloss" x1="14" y1="1" x2="14" y2="15" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#ffffff" stopOpacity="0.3" />
            <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
          </linearGradient>
        </defs>
        <rect x="1" y="1" width="26" height="26" rx="8" fill="url(#ep-tile)" />
        <rect x="1" y="1" width="26" height="26" rx="8" fill="url(#ep-gloss)" />
        <path
          d="M14 5.7l6.1 3.3v6c0 3.6-2.4 6.3-6.1 7.3-3.7-1-6.1-3.7-6.1-7.3V9l6.1-3.3z"
          fill="rgba(5,32,28,0.26)"
        />
        <path
          d="M14 5.7l6.1 3.3v6c0 3.6-2.4 6.3-6.1 7.3-3.7-1-6.1-3.7-6.1-7.3V9l6.1-3.3z"
          stroke="rgba(255,255,255,0.86)"
          strokeWidth="1.35"
          strokeLinejoin="round"
        />
        <circle cx="14" cy="13.3" r="1.85" fill="#f7fffd" />
        <path d="M13.25 15h1.5l0.45 3.05h-2.4z" fill="#f7fffd" />
      </svg>
    </span>
  );
}
