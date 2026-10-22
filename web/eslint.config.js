import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["src/rules-bridge/pkg/**", "src/net/generated/**", "dist/**"] },
  ...tseslint.configs.recommended,
);
