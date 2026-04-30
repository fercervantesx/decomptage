import { tool } from 'ai';
import { z } from 'zod';
import { writeFile, mkdir } from 'fs/promises';
import { dirname } from 'path';

export const writeFileTool = tool({
  description: 'Write content to a file. Creates the file and parent directories if they don\'t exist, overwrites if it does.',
  parameters: z.object({
    path: z.string().describe('Path to write to'),
    content: z.string().describe('The full content to write'),
  }),
  execute: async ({ path, content }) => {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content, 'utf-8');
    return `Successfully wrote ${content.length} bytes to ${path}`;
  },
});
