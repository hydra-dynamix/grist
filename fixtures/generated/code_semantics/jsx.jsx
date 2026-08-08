/** JSX view documentation. */
export class View extends Base {
  run(value) { return <section>{value}</section>; }
}
test("view", () => new View().run("ok"));
