import { writeFileSync } from "node:fs";
import { join } from "node:path";

const output = {
  cwd: process.cwd(),
  cacheDir: process.env.BUN_INSTALL_CACHE_DIR,
  message: "hello from bun sandbox",
};

writeFileSync(
  join(process.cwd(), "sandbox_output.json"),
  `${JSON.stringify(output, null, 2)}\n`,
  "utf8",
);

console.log(output.message);
