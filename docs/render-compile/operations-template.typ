#let ink = rgb("182b45")
#let accent = rgb("167e83")
#let muted = rgb("64748b")
#let manual(title: [], subtitle: [], body) = {
  set document(title: "XUI Render Compile 操作手册", author: "XUI", date: datetime(year: 2026, month: 10, day: 12))
  set page(paper: "a4", margin: (top: 22mm, bottom: 21mm, left: 23mm, right: 21mm),
    header: context if counter(page).get().first() > 1 {
      text(size: 8pt, fill: muted)[XUI / ENGINEERING MANUAL #h(1fr) RENDER COMPILE]
      line(length: 100%, stroke: 0.5pt + rgb("d7e1ea"))
    },
    footer: context {
      text(size: 8pt, fill: muted)[工作区实现 · 2026-10-12 #h(1fr) #counter(page).display("1")]
    })
  set text(font: ("PingFang SC", "Arial"), size: 10pt, lang: "zh", fill: ink)
  set par(justify: true, leading: 0.65em, spacing: 0.85em)
  set heading(numbering: "1.1", outlined: true)
  show heading.where(level: 1): it => block(above: 0pt, below: 14pt)[#text(size: 23pt, weight: "bold", fill: ink)[#it]]
  show heading.where(level: 2): it => block(above: 12pt, below: 7pt)[#text(size: 13pt, weight: "bold", fill: accent)[#it]]
  show raw: set text(font: ("Menlo", "PingFang SC"), size: 8pt)
  show raw.where(block: true): it => block(width: 100%, fill: rgb("f2f6f9"), inset: 10pt, radius: 4pt, breakable: true)[#it]
  set table(inset: 7pt, stroke: 0.5pt + rgb("d7e1ea"), align: left)
  show table.cell: set par(justify: false)
  show table.cell.where(y: 0): set text(weight: "bold", fill: accent)
  body
}
#let note(title, body) = block(width: 100%, fill: rgb("eaf5f3"), inset: 12pt, radius: 4pt)[
  #text(weight: "bold", fill: accent)[#title] #linebreak()
  #body
]
#let chapter(body) = { pagebreak(); body }
