/** Service documentation. */
import helper from "pkg";
export class Service extends Base {
  run(value) {
    const result = helper(value);
    if (result) return result;
  }
}
test("service", () => new Service().run("ok"));
