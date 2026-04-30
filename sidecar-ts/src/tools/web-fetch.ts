import { tool } from 'ai';
import { z } from 'zod';

export const webFetchTool = tool({
  description: 'Fetch content from a URL. Useful for reading documentation, API references, package info. Returns text with HTML stripped.',
  parameters: z.object({
    url: z.string().describe('The URL to fetch'),
    max_length: z.number().optional().describe('Maximum characters to return. Default 10000.'),
  }),
  execute: async ({ url, max_length }) => {
    const maxLen = max_length ?? 10000;
    try {
      const resp = await fetch(url, {
        headers: { 'User-Agent': 'Decomptage/1.0' },
        signal: AbortSignal.timeout(10000),
      });
      if (!resp.ok) return `HTTP error: ${resp.status}`;
      const html = await resp.text();
      const text = stripHtml(html);
      if (text.length > maxLen) {
        return text.slice(0, maxLen) + `\n...[truncated at ${maxLen} chars]`;
      }
      return text;
    } catch (e: any) {
      return `Error fetching URL: ${e.message}`;
    }
  },
});

function stripHtml(html: string): string {
  // Remove script and style blocks
  let text = html.replace(/<script[\s\S]*?<\/script>/gi, '');
  text = text.replace(/<style[\s\S]*?<\/style>/gi, '');
  // Remove HTML tags
  text = text.replace(/<[^>]+>/g, '');
  // Decode common entities
  text = text.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&nbsp;/g, ' ');
  // Collapse whitespace
  text = text.replace(/\n\s*\n\s*\n/g, '\n\n');
  return text.trim();
}
