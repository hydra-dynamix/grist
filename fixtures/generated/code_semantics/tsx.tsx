/** TSX view documentation. */
interface Runnable { run(value: string): JSX.Element; }
export class View extends Base implements Runnable {
  run(value: string): JSX.Element { return <section>{value}</section>; }
}
test("view", () => new View().run("ok"));
