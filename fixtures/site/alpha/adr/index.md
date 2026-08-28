# adr

A section whose whole directory name is an acronym, and the fixture for the
acronym table in `okf-meta.html`'s `title` region. OKF §8 allows front matter
on `index.md` only at a bundle root, so this section arrives with no title at
all and the template falls back to the directory name. `humanize` would render
it "Adr" — on the breadcrumb, on the H1, in every listing entry that links
here, in the browser tab and at the top of this section's `llms.txt`.

<!-- BEGIN OKF INDEX (tools/okf-index) -->
* [Record the acronym](0001-record-the-acronym.md) - The section's only record.
<!-- END OKF INDEX -->
