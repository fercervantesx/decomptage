import { tool } from 'ai';
import { z } from 'zod';
import { execSync } from 'child_process';

export const searchFilesTool = tool({
  description: 'Search for a pattern in files (like grep). Returns matching lines with file paths and line numbers.',
  parameters: z.object({
    pattern: z.string().describe('The text or regex pattern to search for'),
    path: z.string().optional().describe('Directory or file to search in. Defaults to current directory.'),
    file_pattern: z.string().optional().describe("Glob for file names to include (e.g., '*.ts', '*.py')"),
  }),
  execute: async ({ pattern, path, file_pattern }) => {
    const dir = path || '.';
    const includeFlag = file_pattern ? `--include="${file_pattern}"` : '';
    const cmd = `grep -rn ${includeFlag} "${pattern}" "${dir}" 2>/dev/null | head -50`;
    try {
      const output = execSync(cmd, { encoding: 'utf-8', timeout: 10000 });
      return output.trim() || 'No matches found.';
    } catch {
      return 'No matches found.';
    }
  },
});
