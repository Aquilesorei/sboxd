#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const kleur = require("kleur");

const output = {
  cwd: process.cwd(),
  nodePath: process.env.NODE_PATH,
  npmConfigCache: process.env.npm_config_cache,
  npmConfigPrefix: process.env.npm_config_prefix,
  message: "hello from npm sandbox"
};

fs.writeFileSync(
  path.join(process.cwd(), "sandbox_output.json"),
  `${JSON.stringify(output, null, 2)}\n`,
  "utf8"
);

console.log(kleur.green(output.message));
console.log(path.join(process.cwd(), "sandbox_output.json"));
