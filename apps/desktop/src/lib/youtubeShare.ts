export interface AndroidSharePayload {
  text: string;
  subject?: string;
}

export interface ParsedYouTubeShare {
  url: string;
  videoId: string | null;
  title: string;
}

const URL_REGEX = /https?:\/\/[^\s<>"']+/i;

function stripTrailingPunctuation(rawUrl: string): string {
  return rawUrl.replace(/[),.;!?]+$/, '');
}

export function extractYouTubeVideoId(rawUrl: string): string | null {
  try {
    const parsed = new URL(rawUrl);
    const host = parsed.hostname.replace(/^www\./i, '').toLowerCase();
    if (host === 'youtu.be') {
      const seg = parsed.pathname.split('/').filter(Boolean)[0];
      return seg || null;
    }
    if (host === 'youtube.com' || host === 'm.youtube.com' || host === 'music.youtube.com') {
      const vParam = parsed.searchParams.get('v');
      if (vParam) return vParam;
      const parts = parsed.pathname.split('/').filter(Boolean);
      if (parts.length >= 2 && (parts[0] === 'shorts' || parts[0] === 'live' || parts[0] === 'embed')) {
        return parts[1] || null;
      }
    }
  } catch {
    // Fallback si la URL no es parseable por URL()
  }
  return null;
}

export function sanitizeNoteFilename(raw: string): string {
  return raw
    .replace(/[<>:"/\\|?*\x00-\x1F]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/\.+$/, '')
    .slice(0, 90)
    .trim();
}

export function parseSharedYouTubePayload(payload: AndroidSharePayload): ParsedYouTubeShare {
  const rawText = (payload.text ?? '').trim();
  const rawSubject = (payload.subject ?? '').trim();

  const urlMatch = rawText.match(URL_REGEX);
  const url = urlMatch ? stripTrailingPunctuation(urlMatch[0]) : rawText;
  const videoId = url ? extractYouTubeVideoId(url) : null;

  let title = '';
  if (rawSubject && !URL_REGEX.test(rawSubject)) {
    title = rawSubject;
  } else if (urlMatch) {
    const remainder = rawText
      .replace(urlMatch[0], ' ')
      .replace(/\s+/g, ' ')
      .trim();
    if (remainder) {
      title = remainder;
    }
  }

  return { url, videoId, title };
}

export function buildUniqueInboxNotePath(
  inboxFolder: string,
  parsed: ParsedYouTubeShare,
  existingRelativePaths: string[],
  dateStr: string,
  timeStr: string,
): string {
  const folder = (inboxFolder.trim() || 'Inbox').replace(/^\/+|\/+$/g, '') || 'Inbox';
  const safeTitle = parsed.title ? sanitizeNoteFilename(parsed.title) : '';
  const baseName = safeTitle
    ? safeTitle
    : `${dateStr} ${timeStr.replace(/:/g, '-')} YouTube${parsed.videoId ? ` ${parsed.videoId}` : ''}`;

  const existingLower = new Set(existingRelativePaths.map((p) => p.toLowerCase()));
  let candidate = `${folder}/${baseName}.md`;
  let counter = 2;
  while (existingLower.has(candidate.toLowerCase())) {
    candidate = `${folder}/${baseName} (${counter}).md`;
    counter += 1;
  }
  return candidate;
}

function escapeYamlDoubleQuoted(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\r?\n/g, ' ');
}

export function buildYouTubeNoteContent(
  parsed: ParsedYouTubeShare,
  createdDate: string,
): string {
  const displayTitle =
    parsed.title || (parsed.videoId ? `YouTube ${parsed.videoId}` : 'Video de YouTube');
  const safeTitle = escapeYamlDoubleQuoted(displayTitle);
  const safeUrl = escapeYamlDoubleQuoted(parsed.url);

  return [
    '---',
    `title: "${safeTitle}"`,
    'source: youtube',
    `url: "${safeUrl}"`,
    `created: ${createdDate}`,
    'tags:',
    '  - youtube',
    '---',
    '',
    `# ${displayTitle}`,
    '',
    `[Ver en YouTube](${parsed.url})`,
    parsed.url,
    '',
    '## Resumen',
    '',
    '',
  ].join('\n');
}
