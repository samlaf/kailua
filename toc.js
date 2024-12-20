// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item expanded affix "><li class="part-title">Overview</li><li class="chapter-item expanded "><a href="introduction.html"><strong aria-hidden="true">1.</strong> Introduction</a></li><li class="chapter-item expanded "><a href="quickstart.html"><strong aria-hidden="true">2.</strong> Quickstart</a></li><li class="chapter-item expanded "><a href="design.html"><strong aria-hidden="true">3.</strong> Design</a></li><li class="chapter-item expanded "><a href="project.html"><strong aria-hidden="true">4.</strong> Project</a></li><li class="chapter-item expanded affix "><li class="part-title">Migration Guide</li><li class="chapter-item expanded "><a href="setup.html"><strong aria-hidden="true">5.</strong> Setup</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="parameters.html"><strong aria-hidden="true">5.1.</strong> Parameters</a></li></ol></li><li class="chapter-item expanded "><a href="upgrade.html"><strong aria-hidden="true">6.</strong> On-chain</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="verifier.html"><strong aria-hidden="true">6.1.</strong> Proof Verification</a></li><li class="chapter-item expanded "><a href="dispute.html"><strong aria-hidden="true">6.2.</strong> Dispute Resolution</a></li><li class="chapter-item expanded "><a href="treasury.html"><strong aria-hidden="true">6.3.</strong> State Anchoring</a></li><li class="chapter-item expanded "><a href="game.html"><strong aria-hidden="true">6.4.</strong> Sequencing Proposal</a></li></ol></li><li class="chapter-item expanded "><a href="operate.html"><strong aria-hidden="true">7.</strong> Off-chain</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="proposer.html"><strong aria-hidden="true">7.1.</strong> Proposer</a></li><li class="chapter-item expanded "><a href="validator.html"><strong aria-hidden="true">7.2.</strong> Validator</a></li></ol></li><li class="chapter-item expanded "><li class="part-title">Specification</li><li class="chapter-item expanded "><div><strong aria-hidden="true">8.</strong> Sequencing</div></li><li class="chapter-item expanded "><div><strong aria-hidden="true">9.</strong> Validating</div></li><li class="chapter-item expanded affix "><li class="part-title">Implementation</li><li class="chapter-item expanded "><div><strong aria-hidden="true">10.</strong> Prover</div></li><li><ol class="section"><li class="chapter-item expanded "><div><strong aria-hidden="true">10.1.</strong> Host</div></li><li class="chapter-item expanded "><div><strong aria-hidden="true">10.2.</strong> Client</div></li><li class="chapter-item expanded "><div><strong aria-hidden="true">10.3.</strong> FPVM</div></li></ol></li><li class="chapter-item expanded "><div><strong aria-hidden="true">11.</strong> Contracts</div></li><li><ol class="section"><li class="chapter-item expanded "><div><strong aria-hidden="true">11.1.</strong> Treasury</div></li><li class="chapter-item expanded "><div><strong aria-hidden="true">11.2.</strong> Tournament</div></li><li class="chapter-item expanded "><div><strong aria-hidden="true">11.3.</strong> Game</div></li></ol></li><li class="chapter-item expanded "><div><strong aria-hidden="true">12.</strong> CLI</div></li><li><ol class="section"><li class="chapter-item expanded "><div><strong aria-hidden="true">12.1.</strong> Upgrade</div></li><li class="chapter-item expanded "><div><strong aria-hidden="true">12.2.</strong> Propose</div></li><li class="chapter-item expanded "><div><strong aria-hidden="true">12.3.</strong> Validate</div></li><li class="chapter-item expanded "><div><strong aria-hidden="true">12.4.</strong> Fault</div></li></ol></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString();
        if (current_page.endsWith("/")) {
            current_page += "index.html";
        }
        var links = Array.prototype.slice.call(this.querySelectorAll("a"));
        var l = links.length;
        for (var i = 0; i < l; ++i) {
            var link = links[i];
            var href = link.getAttribute("href");
            if (href && !href.startsWith("#") && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The "index" page is supposed to alias the first chapter in the book.
            if (link.href === current_page || (i === 0 && path_to_root === "" && current_page.endsWith("/index.html"))) {
                link.classList.add("active");
                var parent = link.parentElement;
                if (parent && parent.classList.contains("chapter-item")) {
                    parent.classList.add("expanded");
                }
                while (parent) {
                    if (parent.tagName === "LI" && parent.previousElementSibling) {
                        if (parent.previousElementSibling.classList.contains("chapter-item")) {
                            parent.previousElementSibling.classList.add("expanded");
                        }
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', function(e) {
            if (e.target.tagName === 'A') {
                sessionStorage.setItem('sidebar-scroll', this.scrollTop);
            }
        }, { passive: true });
        var sidebarScrollTop = sessionStorage.getItem('sidebar-scroll');
        sessionStorage.removeItem('sidebar-scroll');
        if (sidebarScrollTop) {
            // preserve sidebar scroll position when navigating via links within sidebar
            this.scrollTop = sidebarScrollTop;
        } else {
            // scroll sidebar to current active section when navigating via "next/previous chapter" buttons
            var activeSection = document.querySelector('#sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        var sidebarAnchorToggles = document.querySelectorAll('#sidebar a.toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(function (el) {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define("mdbook-sidebar-scrollbox", MDBookSidebarScrollbox);
