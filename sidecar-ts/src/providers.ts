import { anthropic } from '@ai-sdk/anthropic';
import { openai } from '@ai-sdk/openai';
import { google } from '@ai-sdk/google';
import { bedrock } from '@ai-sdk/amazon-bedrock';
import { ollama } from 'ollama-ai-provider';

export function getProvider(id: string, model: string) {
  switch (id) {
    case 'anthropic':
      return anthropic(model);
    case 'openai':
      return openai(model);
    case 'gemini':
      return google(model);
    case 'bedrock':
      return bedrock(model);
    case 'ollama':
      return ollama(model);
    default:
      throw new Error(`Unknown provider: ${id}`);
  }
}

export async function getAvailableProviders() {
  return [
    {
      id: 'anthropic',
      models: ['claude-opus-4-7', 'claude-sonnet-4-6', 'claude-haiku-4-5'],
      streaming: true,
      vision: true,
      configured: !!process.env.ANTHROPIC_API_KEY,
    },
    {
      id: 'openai',
      models: ['gpt-4o', 'gpt-4o-mini', 'o3'],
      streaming: true,
      vision: true,
      configured: !!process.env.OPENAI_API_KEY,
    },
    {
      id: 'gemini',
      models: ['gemini-2.5-pro-preview-05-06', 'gemini-2.5-flash-preview-04-17'],
      streaming: true,
      vision: true,
      configured: !!process.env.GOOGLE_GENERATIVE_AI_API_KEY,
    },
    {
      id: 'bedrock',
      models: ['anthropic.claude-sonnet-4-6-20250514-v1:0', 'anthropic.claude-haiku-4-5-20251001-v1:0', 'us.anthropic.claude-opus-4-7-20250506-v1:0'],
      streaming: true,
      vision: true,
      configured: !!process.env.AWS_ACCESS_KEY_ID || !!process.env.AWS_PROFILE,
    },
    {
      id: 'ollama',
      models: await fetchOllamaModels(),
      streaming: true,
      vision: false,
      configured: true,
    },
  ];
}

async function fetchOllamaModels(): Promise<string[]> {
  const url = process.env.OLLAMA_URL || 'http://localhost:11434';
  try {
    const resp = await fetch(`${url}/api/tags`);
    const data = await resp.json() as { models?: { name: string }[] };
    return data.models?.map(m => m.name) ?? ['llama3.3', 'gemma4'];
  } catch {
    return ['llama3.3', 'gemma4'];
  }
}
