import js from '@eslint/js';
import globals from 'globals';
import reactHooks from 'eslint-plugin-react-hooks';
import tseslint from 'typescript-eslint';

// DESIGN.md §11 rule 1: hex colours live only in src/theme.ts.
const HEX = '^#([0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$';
const hexRules = [
  {
    selector: `Literal[value=/${HEX}/]`,
    message: 'No raw hex colours outside theme.ts. Use var(--dl-*) tokens or Mantine colour keys.',
  },
  {
    selector: 'TemplateElement[value.raw=/#([0-9a-fA-F]{6}|[0-9a-fA-F]{3})\\b/]',
    message: 'No raw hex colours outside theme.ts. Use var(--dl-*) tokens or Mantine colour keys.',
  },
  {
    selector:
      'JSXAttribute[name.name=/^(style|sx)$/] Property[key.name=/(?i:color|background|border|shadow|fill|stroke)/] > Literal[value=/^(#|rgb|hsl)/]',
    message: 'No raw colours in style/sx props. Use var(--dl-*) tokens.',
  },
];

export default tseslint.config(
  { ignores: ['dist', 'storybook-static', 'node_modules', 'playwright-report', 'test-results'] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: { ecmaVersion: 2023, globals: { ...globals.browser, ...globals.node } },
    plugins: { 'react-hooks': reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      'no-restricted-syntax': ['error', ...hexRules],
      '@typescript-eslint/consistent-type-imports': 'error',
    },
  },
  {
    // The only place hex literals are allowed.
    files: ['src/theme.ts'],
    rules: { 'no-restricted-syntax': 'off' },
  },
);
