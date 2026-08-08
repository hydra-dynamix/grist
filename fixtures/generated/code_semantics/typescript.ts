/** Service documentation. */
import { helper } from "pkg";
export interface Runnable { run(value: string): string; }
export class Service extends Base implements Runnable {
  run(value: string): string {
    const result = helper(value);
    if (result) return result;
    return value;
  }
}
export function invoke(value: string): string { return run(value); }
test("service", () => new Service().run("ok"));
