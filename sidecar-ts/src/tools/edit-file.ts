import { tool } from 'ai';
import { z } from 'zod';
import { readFile, writeFile } from 'fs/promises';

export const editFileTool = tool({
  description: 'Make a targeted edit to a file by replacing a specific string. The old_string must match exactly (including whitespace) and be unique in the file.',
  parameters: z.object({
    path: z.string().describe('Path to the file to edit'),
    old_string: z.string().describe('The exact string to find and replace (must be unique)'),
    new_string: z.string().describe('The string to replace it with'),
  }),
  execute: async ({ path, old_string, new_string }) => {
    const content = await readFile(path, 'utf-8');
    const count = content.split(old_string).length - 1;
    if (count === 0) return `Error: old_string not found in ${path}`;
    if (count > 1) return `Error: old_string found ${count} times in ${path} (must be unique)`;
    const newContent = content.replace(old_string, new_string);
    await writeFile(path, newContent, 'utf-8');
    return `Successfully edited ${path}`;
  },
});
