module.exports = {
  root: true,
  extends: '@react-native',
  ignorePatterns: ['android/', 'ios/', 'coverage/'],
  rules: {
    'react-native/no-inline-styles': 'off',
    'no-console': ['warn', {allow: ['warn', 'error']}],
  },
  overrides: [
    {
      // The dev logger is the one place allowed to write to the console.
      files: ['src/lib/logger.ts'],
      rules: {'no-console': 'off'},
    },
  ],
};
