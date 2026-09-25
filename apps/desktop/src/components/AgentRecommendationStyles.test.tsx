import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import css from "../App.css?raw";
import { DisplaySummary } from "./DisplaySummary";


describe("recommendation card layout", () => {
  it("keeps summary spans out of score styling and reserves a full-width heading", () => {
    const { container } = render(<>
      <style>{css}</style>
      <section className="agent-recommendations">
        <div className="agent-recommendations__list">
          {["51%", "手动加入", "Manually added"].map((label) => <article key={label}>
            <div className="agent-recommendations__item-heading">
              <strong>game-design-theory-with-a-long-name</strong>
              <span className="agent-recommendations__item-status">{label}</span>
              <button className="agent-recommendations__remove">×</button>
            </div>
            <p><DisplaySummary text={`概述 ${label}`} /></p>
          </article>)}
        </div>
      </section>
    </>);
    for (const heading of container.querySelectorAll(".agent-recommendations__item-heading")) {
      const style = getComputedStyle(heading);
      expect(style.display).toBe("grid");
      expect(style.gridTemplateColumns).toBe("minmax(0, 1fr) auto 26px");
      expect(style.width).toBe("100%");
      expect(getComputedStyle(heading.querySelector("strong")!).whiteSpace).toBe("nowrap");
    }
    for (const label of ["51%", "手动加入", "Manually added"]) {
      const summary = screen.getByText(`概述 ${label}`);
      expect(summary.tagName).toBe("SPAN");
      expect(getComputedStyle(summary).cssFloat).not.toBe("right");
      expect(getComputedStyle(summary).color).not.toBe(getComputedStyle(screen.getByText(label)).color);
    }
  });
});
