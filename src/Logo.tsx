/**
 * The app mark: the edgePass candado — a ring and a body crossed by the edge,
 * with the keyhole cut out so the tile behind it shows through.
 *
 * Same drawing as `docs/assets/brand/candado.svg`: no filter, no painted
 * keyhole, no live text, so it stays crisp at 16 px and on any background.
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
      <svg viewBox="0 0 512 512" fill="none" xmlns="http://www.w3.org/2000/svg">
        <defs>
          <mask id="ep-keyhole" maskUnits="userSpaceOnUse" x="0" y="0" width="512" height="512">
            <rect width="512" height="512" fill="#fff" />
            <g transform="translate(256 256)" fill="#000">
              <circle cy="-7" r="16" />
              <path d="M-7 -4 L7 -4 L14 32 L-14 32 Z" />
            </g>
          </mask>
        </defs>
        <g mask="url(#ep-keyhole)">
          <g transform="translate(256 256)">
            <g transform="rotate(-45)">
              <rect
                x="-76"
                y="-124"
                width="152"
                height="248"
                rx="76"
                stroke="#00E5FF"
                strokeWidth="20"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <rect x="-52" y="-100" width="104" height="200" rx="52" fill="#79FBBD" />
            </g>
            <g transform="rotate(45)">
              <rect x="-124" y="-15" width="248" height="30" rx="15" fill="#79FBBD" />
            </g>
          </g>
        </g>
      </svg>
    </span>
  );
}
