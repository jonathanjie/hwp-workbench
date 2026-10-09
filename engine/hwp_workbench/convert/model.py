"""Small conversion model. Geometry uses DOCX twips (1/1440 inch)."""
from dataclasses import dataclass, field, replace


@dataclass
class Image:
    data: bytes
    media_type: str
    width: int = 1440
    height: int = 1440
    name: str = 'Image'


@dataclass
class Run:
    text: str = ''
    style: dict = field(default_factory=dict)
    image: Image | None = None


@dataclass
class Paragraph:
    runs: list[Run] = field(default_factory=list)
    style: dict = field(default_factory=dict)
    heading: int = 0
    numbering: dict | None = None
    page_break: bool = False

    @property
    def text(self):
        return ''.join(run.text for run in self.runs)


@dataclass
class Cell:
    blocks: list = field(default_factory=list)
    column: int = 0
    colspan: int = 1
    rowspan: int = 1
    borders: dict = field(default_factory=dict)
    shading: str | None = None


@dataclass
class Table:
    rows: list[list[Cell]] = field(default_factory=list)
    widths: list[int] = field(default_factory=list)


DEFAULT_PAGE = dict(width=11906, height=16838, orientation='portrait', left=1440,
                    right=1440, top=1440, bottom=1440, header=720, footer=720, gutter=0)


@dataclass
class Section:
    blocks: list = field(default_factory=list)
    page: dict = field(default_factory=lambda: dict(DEFAULT_PAGE))


def paragraphs(blocks):
    for block in blocks:
        if isinstance(block, Paragraph):
            yield block
        elif isinstance(block, Table):
            for row in block.rows:
                for cell in row:
                    yield from paragraphs(cell.blocks)


@dataclass
class RichDocument:
    sections: list[Section] = field(default_factory=list)
    unsupported: dict[str, int] = field(default_factory=dict)

    def loss(self, feature, count=1):
        if count:
            self.unsupported[feature] = self.unsupported.get(feature, 0) + count

    def paragraph_texts(self):
        return [paragraph.text for section in self.sections for paragraph in paragraphs(section.blocks)]


def split_page_breaks(blocks):
    """Native page breaks start a paragraph; split OOXML inline breaks there."""
    result = []
    for block in blocks:
        if isinstance(block, Table):
            for row in block.rows:
                for cell in row:
                    cell.blocks = split_page_breaks(cell.blocks)
            result.append(block)
            continue
        current = replace(block, runs=[])
        for run in block.runs:
            pieces = run.text.split('\f')
            for index, text in enumerate(pieces):
                if index:
                    result.append(current)
                    current = replace(block, runs=[], page_break=True, numbering=None, heading=0)
                current.runs.append(replace(run, text=text, image=run.image if index == 0 else None))
        result.append(current)
    return result
