import type { Config } from "./config";

export interface Options {
  name: string;
  count?: number;
}

export function describe(options: Options, config: Config): string {
  const count = options.count ?? 0;
  return `${options.name}: ${count} of ${config.limit}`;
}
