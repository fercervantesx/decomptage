import { tool } from 'ai';
import { z } from 'zod';
import { readFile } from 'fs/promises';

export const readFileTool = tool({
  description: 'Read the contents of a file. Use for source code, configs, docs, READMEs, etc.',
  parameters: z.object({
    path: z.string().describe('Path to the file (relative to cwd or absolute)'),
    start_line: z.number().optional().describe('Line to start reading from (1-based)'),
    end_line: z.number().optional().describe('Line to stop reading at (inclusive)'),
  }),
  execute: async ({ path, start_line, end_line }) => {
    const content = await readFile(path, 'utf-8');
    const lines = content.split('\n');
    const start = (start_line ?? 1) - 1;
    const end = end_line ?? Math.min(lines.length, start + 1000);
    return lines
      .slice(start, end)
      .map((l, i) => `${start + i + 1} | ${l}`)
      .join('\n');
  },
});
