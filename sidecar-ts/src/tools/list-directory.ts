import { tool } from 'ai';
import { z } from 'zod';
import { readdir } from 'fs/promises';
import { execSync } from 'child_process';

export const listDirectoryTool = tool({
  description: 'List files and directories at a path. Returns names with / suffix for directories.',
  parameters: z.object({
    path: z.string().optional().describe('Directory path. Defaults to current directory.'),
    recursive: z.boolean().optional().describe('If true, list recursively (max 3 levels). Default false.'),
  }),
  execute: async ({ path, recursive }) => {
    const dir = path || '.';
    if (recursive) {
      try {
        const output = execSync(`find "${dir}" -maxdepth 3 \\( -type f -o -type d \\) | head -200`, {
          encoding: 'utf-8',
          timeout: 5000,
        });
        return output.trim();
      } catch (e: any) {
        return `Error: ${e.message}`;
      }
    }
    const entries = await readdir(dir, { withFileTypes: true });
    return entries
      .map(e => e.isDirectory() ? `${e.name}/` : e.name)
      .sort()
      .join('\n');
  },
});
