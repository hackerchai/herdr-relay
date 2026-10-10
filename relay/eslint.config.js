import js from '@eslint/js';
import ts from 'typescript-eslint';

export default ts.config(
  { ignores: ['node_modules/**', '.wrangler/**', 'dist/**'] },
  js.configs.recommended,
  ...ts.configs.recommended,
  {
    files: ['src/**/*.ts', 'test/**/*.ts'],
    rules: {
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
      '@typescript-eslint/consistent-type-imports': 'error',
    },
  },
);
