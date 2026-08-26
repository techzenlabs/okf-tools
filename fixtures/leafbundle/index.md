# Leaf bundle

Five markdown files sit in this directory. OKF §8 says this one is the
listing; Hugo says a directory holding an `index.md` is a *leaf bundle*, which
demotes the other four from pages to page resources of this one.

The fixture beside it renders this directory twice — once as written and once
with the rename applied — and asserts both page counts, because the failure is
a successful build with most of the corpus missing from it.

<!-- BEGIN OKF INDEX -->
* [Alpha](alpha.md) - One.
* [Beta](beta.md) - Two.
* [Gamma](gamma.md) - Three.
* [Delta](delta.md) - Four.
<!-- END OKF INDEX -->
