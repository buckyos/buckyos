import type {
  FeedAuthor,
  FeedFilter,
  FeedObject,
  Source,
  Topic,
  UserProfile,
} from '../types'

/* ── Authors ── */

export const mockAuthors: FeedAuthor[] = [
  { id: 'author-1', name: 'Alice Chen', sourceType: 'did', isVerified: true },
  { id: 'author-2', name: 'Bob Zhang', sourceType: 'platform' },
  { id: 'author-3', name: 'TechCrunch', sourceType: 'rss', isVerified: true },
  { id: 'author-4', name: 'AI Daily Digest', sourceType: 'agent' },
  { id: 'author-5', name: 'Sarah Kim', sourceType: 'did', isVerified: true },
  { id: 'author-6', name: 'Local SF News', sourceType: 'rss' },
  { id: 'author-7', name: 'David Liu', sourceType: 'platform' },
  { id: 'author-8', name: 'Foodie Explorer', sourceType: 'did' },
]

/* ── Topics ── */

export const mockTopics: Topic[] = [
  { id: 'topic-tech', name: 'Tech', feedCount: 128, isSubscribed: true, trendScore: 92 },
  { id: 'topic-ai', name: 'AI & ML', feedCount: 85, isSubscribed: true, trendScore: 98 },
  { id: 'topic-local', name: 'Local News', feedCount: 42, isSubscribed: true, trendScore: 65 },
  { id: 'topic-food', name: 'Food & Cooking', feedCount: 67, isSubscribed: false, trendScore: 55 },
  { id: 'topic-finance', name: 'Finance', feedCount: 93, isSubscribed: true, trendScore: 78 },
  { id: 'topic-photography', name: 'Photography', feedCount: 51, isSubscribed: false, trendScore: 40 },
  { id: 'topic-gaming', name: 'Gaming', feedCount: 76, isSubscribed: false, trendScore: 60 },
  { id: 'topic-design', name: 'Design', feedCount: 38, isSubscribed: true, trendScore: 45 },
]

/* ── Sources ── */

export const mockSources: Source[] = [
  { id: 'src-1', name: 'Alice Chen', type: 'person', isFollowing: true, feedCount: 34, lastUpdatedAt: Date.now() - 3600_000, description: 'Full-stack developer & open source contributor' },
  { id: 'src-2', name: 'TechCrunch', type: 'rss', url: 'https://techcrunch.com/feed/', isFollowing: true, feedCount: 210, lastUpdatedAt: Date.now() - 1800_000, description: 'Startup and technology news' },
  { id: 'src-3', name: 'r/programming', type: 'channel', isFollowing: true, feedCount: 156, lastUpdatedAt: Date.now() - 7200_000, description: 'Programming subreddit' },
  { id: 'src-4', name: 'AI News Agent', type: 'agent-curated', isFollowing: true, feedCount: 89, lastUpdatedAt: Date.now() - 900_000, description: 'AI-curated daily digest of AI/ML developments' },
  { id: 'src-5', name: 'Sarah Kim', type: 'person', isFollowing: true, feedCount: 22, lastUpdatedAt: Date.now() - 14400_000, description: 'Product designer at BuckyOS' },
  { id: 'src-6', name: 'Hacker News', type: 'website', url: 'https://news.ycombinator.com', isFollowing: true, feedCount: 340, lastUpdatedAt: Date.now() - 600_000, description: 'Tech news aggregator' },
  { id: 'src-7', name: 'SF Local', type: 'topic', isFollowing: true, feedCount: 45, lastUpdatedAt: Date.now() - 5400_000, description: 'San Francisco local news and events' },
  { id: 'src-8', name: 'David Liu', type: 'person', isFollowing: false, feedCount: 18, lastUpdatedAt: Date.now() - 28800_000, description: 'Indie game developer' },
  { id: 'src-9', name: 'Bloomberg Markets', type: 'rss', url: 'https://bloomberg.com', isFollowing: true, feedCount: 175, lastUpdatedAt: Date.now() - 2400_000, description: 'Financial news and analysis' },
  { id: 'src-10', name: 'Cooking with AI', type: 'agent-curated', isFollowing: false, feedCount: 63, lastUpdatedAt: Date.now() - 43200_000, description: 'AI-curated recipes and food trends' },
]

/* ── User Profile ── */

export const mockUserProfile: UserProfile = {
  id: 'user-self',
  name: 'Leo Wang',
  bio: 'Builder of decentralized systems. Exploring the frontier of personal computing and AI-native experiences.',
  followerCount: 1284,
  followingCount: 326,
  postCount: 89,
}

/* ── Feed Objects ── */

const now = Date.now()
const hour = 3600_000

export const mockFeedObjects: FeedObject[] = [
  {
    id: 'feed-1',
    author: mockAuthors[0],
    contentType: 'text',
    text: 'Just shipped the new decentralized identity module for BuckyOS. The DID resolver now handles cross-zone verification in under 200ms. Huge milestone for the team! 🚀',
    media: [],
    recommendReason: 'Because you follow Alice Chen',
    topics: ['topic-tech'],
    interactions: { likeCount: 42, commentCount: 8, repostCount: 5, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 0.5 * hour,
    sourceId: 'src-1',
  },
  {
    id: 'feed-2',
    author: mockAuthors[2],
    contentType: 'article',
    title: 'OpenAI announces GPT-5 with native multimodal reasoning',
    text: 'OpenAI has unveiled its latest model, GPT-5, featuring breakthrough capabilities in multimodal reasoning, real-time tool use, and significantly improved factual accuracy...',
    body: 'OpenAI has unveiled its latest model, GPT-5, featuring breakthrough capabilities in multimodal reasoning, real-time tool use, and significantly improved factual accuracy. The model demonstrates a 40% improvement on complex reasoning benchmarks compared to its predecessor.\n\nKey highlights include:\n- Native vision, audio, and code understanding in a single model\n- Real-time web browsing and tool integration\n- Significantly reduced hallucination rates\n- New fine-tuning API for enterprise customers\n\nThe announcement comes amid increasing competition from Anthropic, Google, and open-source alternatives. Industry analysts suggest this release could reshape enterprise AI adoption patterns.',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 1200, height: 630, alt: 'GPT-5 announcement banner' },
    ],
    originalUrl: 'https://techcrunch.com/2026/04/03/openai-gpt5',
    recommendReason: 'Trending in AI & ML',
    topics: ['topic-ai', 'topic-tech'],
    interactions: { likeCount: 1205, commentCount: 342, repostCount: 289, isLiked: true, isBookmarked: true, isReposted: false },
    createdAt: now - 1.5 * hour,
    sourceId: 'src-2',
  },
  {
    id: 'feed-3',
    author: mockAuthors[4],
    contentType: 'image',
    text: 'New design explorations for the BuckyOS HomeStation feed card system. Playing with information density and visual hierarchy. Thoughts?',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1350, alt: 'Feed card design v1' },
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1350, alt: 'Feed card design v2' },
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1350, alt: 'Feed card design v3' },
    ],
    recommendReason: 'Because you follow Sarah Kim',
    topics: ['topic-design'],
    interactions: { likeCount: 67, commentCount: 14, repostCount: 8, isLiked: false, isBookmarked: true, isReposted: false },
    createdAt: now - 3 * hour,
    sourceId: 'src-5',
  },
  {
    id: 'feed-4',
    author: mockAuthors[3],
    contentType: 'text',
    text: 'Daily AI Digest: Anthropic releases Claude 4.6 Opus with improved coding capabilities. Meta open-sources new video generation model. DeepMind publishes paper on AI safety evaluation frameworks.',
    media: [],
    recommendReason: 'From your AI News Agent subscription',
    topics: ['topic-ai'],
    interactions: { likeCount: 89, commentCount: 23, repostCount: 31, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 4 * hour,
    sourceId: 'src-4',
  },
  {
    id: 'feed-5',
    author: mockAuthors[5],
    contentType: 'link',
    title: 'San Francisco approves new affordable housing project in SoMa district',
    text: 'The SF Board of Supervisors voted 8-3 to approve a mixed-use development that will include 200 affordable housing units along with retail space and a community center.',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 800, height: 450, alt: 'SoMa development rendering' },
    ],
    originalUrl: 'https://sfchronicle.com/housing/soma-development',
    recommendReason: 'Matches your interest in Local News',
    topics: ['topic-local'],
    interactions: { likeCount: 156, commentCount: 87, repostCount: 42, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 5 * hour,
    sourceId: 'src-7',
  },
  {
    id: 'feed-6',
    author: mockAuthors[1],
    contentType: 'video',
    title: 'Building a personal AI assistant with BuckyOS SDK',
    text: 'Step-by-step tutorial on creating your own AI assistant that runs on your personal server. Covers setup, configuration, and integration with the MessageHub API.',
    media: [
      { type: 'video', url: '', thumbnailUrl: '', width: 1920, height: 1080, durationMs: 845_000, alt: 'Tutorial video thumbnail' },
    ],
    recommendReason: 'Popular in Tech',
    topics: ['topic-tech', 'topic-ai'],
    interactions: { likeCount: 234, commentCount: 45, repostCount: 78, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 6 * hour,
    sourceId: 'src-3',
  },
  {
    id: 'feed-7',
    author: mockAuthors[7],
    contentType: 'image',
    text: 'Made homemade ramen from scratch today! The broth took 12 hours but was absolutely worth it. Recipe thread below 🍜',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1080, alt: 'Homemade ramen bowl' },
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1080, alt: 'Broth preparation' },
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1080, alt: 'Noodle making' },
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1080, alt: 'Final plating' },
    ],
    recommendReason: 'Trending in Food & Cooking',
    topics: ['topic-food'],
    interactions: { likeCount: 312, commentCount: 56, repostCount: 93, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 8 * hour,
    sourceId: 'src-10',
  },
  {
    id: 'feed-8',
    author: mockAuthors[0],
    contentType: 'article',
    title: 'Why Personal Servers Will Replace Cloud Subscriptions',
    text: 'An essay on the economics and philosophy of personal computing infrastructure. As hardware costs drop and AI capabilities increase, the case for personal servers becomes increasingly compelling.',
    body: 'The cloud computing paradigm has dominated for over a decade, but several converging trends suggest a fundamental shift is coming.\n\nFirst, the cost of capable hardware has dropped dramatically. A $500 device can now run multiple AI models, serve web applications, and store terabytes of personal data.\n\nSecond, privacy regulations are making cloud services increasingly complex and expensive to operate. GDPR, CCPA, and emerging AI governance frameworks create compliance overhead that personal servers simply avoid.\n\nThird, the AI revolution means your personal data is now your most valuable asset. Keeping it on your own hardware isn\'t just a privacy choice — it\'s an economic one.\n\nIn this essay, I explore the technical, economic, and philosophical arguments for why the next decade will see a renaissance in personal computing infrastructure.',
    media: [],
    originalUrl: 'https://alice.personal.server/blog/personal-servers',
    recommendReason: 'Because you follow Alice Chen',
    topics: ['topic-tech'],
    interactions: { likeCount: 523, commentCount: 124, repostCount: 187, isLiked: true, isBookmarked: false, isReposted: false },
    createdAt: now - 12 * hour,
    sourceId: 'src-1',
  },
  {
    id: 'feed-9',
    author: mockAuthors[6],
    contentType: 'video',
    title: 'Indie game devlog #24: Procedural world generation',
    text: 'Showing off the new procedural terrain system. Mountains, rivers, and biomes all generated from a single seed value. The performance is surprisingly good even on mid-range hardware.',
    media: [
      { type: 'video', url: '', thumbnailUrl: '', width: 1920, height: 1080, durationMs: 632_000, alt: 'Procedural world demo' },
    ],
    topics: ['topic-gaming'],
    interactions: { likeCount: 178, commentCount: 34, repostCount: 21, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 14 * hour,
    sourceId: 'src-8',
  },
  {
    id: 'feed-10',
    author: mockAuthors[2],
    contentType: 'article',
    title: 'NVIDIA stock surges 12% after record quarterly earnings',
    text: 'NVIDIA reported Q1 2026 revenue of $48.2 billion, beating analyst estimates by 15%. The company\'s data center division drove growth, with AI infrastructure demand showing no signs of slowing.',
    body: 'NVIDIA Corporation reported record-breaking first quarter results for fiscal year 2026, with total revenue reaching $48.2 billion — a 65% increase year-over-year and 15% above Wall Street consensus estimates.\n\nThe data center segment generated $38.1 billion in revenue, representing 79% of total sales. CEO Jensen Huang attributed the growth to "insatiable demand for AI computing infrastructure across every industry."\n\nKey metrics:\n- Gross margin: 78.2% (up from 76.5% YoY)\n- Data center revenue: $38.1B (+72% YoY)\n- Gaming revenue: $6.8B (+18% YoY)\n- Professional visualization: $2.1B (+31% YoY)\n\nThe company also announced a new generation of Blackwell Ultra chips, expected to ship in Q3 2026.',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 1200, height: 675, alt: 'NVIDIA earnings chart' },
    ],
    originalUrl: 'https://techcrunch.com/2026/04/02/nvidia-q1-earnings',
    recommendReason: 'Trending in Finance',
    topics: ['topic-finance', 'topic-tech'],
    interactions: { likeCount: 892, commentCount: 267, repostCount: 445, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 16 * hour,
    sourceId: 'src-2',
  },
  {
    id: 'feed-11',
    author: mockAuthors[4],
    contentType: 'text',
    text: 'Design tip: When building feed systems, always ensure the recommendation reason is visually distinct from the content itself. Users need to immediately understand WHY they\'re seeing something, not just WHAT they\'re seeing.',
    media: [],
    recommendReason: 'Because you follow Sarah Kim',
    topics: ['topic-design'],
    interactions: { likeCount: 145, commentCount: 32, repostCount: 67, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 18 * hour,
    sourceId: 'src-5',
  },
  {
    id: 'feed-12',
    author: mockAuthors[5],
    contentType: 'link',
    title: 'New BART extension to San Jose opens ahead of schedule',
    text: 'The long-awaited BART Silicon Valley Phase II extension opened to riders today, connecting Milpitas, downtown San Jose, and Santa Clara stations.',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 960, height: 540, alt: 'New BART station interior' },
    ],
    originalUrl: 'https://sfgate.com/bart-san-jose-extension',
    recommendReason: 'Matches your interest in Local News',
    topics: ['topic-local'],
    interactions: { likeCount: 432, commentCount: 156, repostCount: 198, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 20 * hour,
    sourceId: 'src-7',
  },
  {
    id: 'feed-13',
    author: mockAuthors[3],
    contentType: 'text',
    text: 'Weekly AI Summary: This week saw major releases from 3 frontier labs, 2 new open-source models broke into the top 10 on benchmarks, and the EU finalized its AI Act enforcement timeline. Full digest in your feed.',
    media: [],
    recommendReason: 'From your AI News Agent subscription',
    topics: ['topic-ai'],
    interactions: { likeCount: 67, commentCount: 12, repostCount: 28, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 24 * hour,
    sourceId: 'src-4',
  },
  {
    id: 'feed-14',
    author: mockAuthors[1],
    contentType: 'image',
    text: 'Golden Gate Bridge at sunset, captured from Lands End trail. The fog was rolling in perfectly. Shot on Pixel 9 Pro.',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 2048, height: 1365, alt: 'Golden Gate Bridge at sunset' },
    ],
    topics: ['topic-photography', 'topic-local'],
    interactions: { likeCount: 567, commentCount: 43, repostCount: 112, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 28 * hour,
    sourceId: 'src-3',
  },
  {
    id: 'feed-15',
    author: mockAuthors[7],
    contentType: 'article',
    title: 'The Complete Guide to Sourdough Bread',
    text: 'After 3 years of baking sourdough, I\'ve compiled everything I\'ve learned into a comprehensive guide. From starter maintenance to shaping techniques, this covers it all.',
    body: 'Sourdough bread baking is both an art and a science. In this guide, I share everything I\'ve learned from three years of daily baking.\n\nChapter 1: Understanding Your Starter\nA healthy starter is the foundation of great sourdough. Feed it equal parts flour and water by weight, keep it at room temperature, and be patient.\n\nChapter 2: The Autolyse Method\nMixing flour and water before adding your starter and salt allows the gluten to develop naturally, resulting in better texture.\n\nChapter 3: Bulk Fermentation\nThis is where the magic happens. Temperature and time are your two most important variables.\n\nChapter 4: Shaping\nGentle handling and proper surface tension are key to achieving that perfect oven spring.\n\nChapter 5: Baking\nDutch oven at 500°F for 20 minutes covered, then 25 minutes uncovered at 450°F.',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1350, alt: 'Fresh sourdough loaf' },
      { type: 'image', url: '', thumbnailUrl: '', width: 1080, height: 1080, alt: 'Crumb shot' },
    ],
    originalUrl: 'https://foodie-explorer.personal/sourdough-guide',
    recommendReason: 'Popular in Food & Cooking',
    topics: ['topic-food'],
    interactions: { likeCount: 723, commentCount: 189, repostCount: 312, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 32 * hour,
    sourceId: 'src-10',
  },
  {
    id: 'feed-16',
    author: mockAuthors[6],
    contentType: 'product',
    title: 'Indie Game: Echoes of the Void',
    text: 'My indie game is finally available for early access! A roguelike exploration game with procedural worlds. $14.99 for early supporters. All proceeds go directly to development.',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 1920, height: 1080, alt: 'Game screenshot - alien landscape' },
      { type: 'image', url: '', thumbnailUrl: '', width: 1920, height: 1080, alt: 'Game screenshot - base building' },
    ],
    originalUrl: 'https://david-liu.personal/games/echoes-of-the-void',
    topics: ['topic-gaming'],
    interactions: { likeCount: 89, commentCount: 23, repostCount: 45, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 36 * hour,
    sourceId: 'src-8',
  },
  {
    id: 'feed-17',
    author: mockAuthors[0],
    contentType: 'video',
    title: 'BuckyOS Demo: Setting up your personal server in 5 minutes',
    text: 'Quick walkthrough showing how to get BuckyOS running on a mini PC. From unboxing to having your personal cloud ready.',
    media: [
      { type: 'video', url: '', thumbnailUrl: '', width: 1920, height: 1080, durationMs: 312_000, alt: 'BuckyOS setup demo' },
    ],
    recommendReason: 'Because you follow Alice Chen',
    topics: ['topic-tech'],
    interactions: { likeCount: 345, commentCount: 67, repostCount: 123, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 40 * hour,
    sourceId: 'src-1',
  },
  {
    id: 'feed-18',
    author: mockAuthors[2],
    contentType: 'link',
    title: 'Bitcoin breaks $150k as institutional adoption accelerates',
    text: 'Bitcoin reached a new all-time high of $152,340 as major banks announce custody solutions and new ETF products see record inflows.',
    media: [
      { type: 'image', url: '', thumbnailUrl: '', width: 1200, height: 630, alt: 'Bitcoin price chart' },
    ],
    originalUrl: 'https://techcrunch.com/2026/04/01/bitcoin-150k',
    recommendReason: 'Trending in Finance',
    topics: ['topic-finance'],
    interactions: { likeCount: 1567, commentCount: 534, repostCount: 678, isLiked: false, isBookmarked: false, isReposted: false },
    createdAt: now - 44 * hour,
    sourceId: 'src-9',
  },
]

/* ── Filter Utility ── */

export function filterFeedObjects(
  items: FeedObject[],
  filter: FeedFilter,
  topicId: string | null,
): FeedObject[] {
  let filtered = items

  if (filter === 'following') {
    const followingSourceIds = new Set(
      mockSources.filter((s) => s.isFollowing).map((s) => s.id),
    )
    filtered = filtered.filter((f) => followingSourceIds.has(f.sourceId))
  } else if (filter === 'images') {
    filtered = filtered.filter(
      (f) => f.contentType === 'image' || f.media.some((m) => m.type === 'image'),
    )
  } else if (filter === 'videos') {
    filtered = filtered.filter(
      (f) => f.contentType === 'video' || f.media.some((m) => m.type === 'video'),
    )
  } else if (filter === 'longform') {
    filtered = filtered.filter(
      (f) => f.contentType === 'article' && f.body,
    )
  } else if (filter === 'news') {
    filtered = filtered.filter(
      (f) => f.contentType === 'article' || f.contentType === 'link',
    )
  }

  if (topicId) {
    filtered = filtered.filter((f) => f.topics.includes(topicId))
  }

  return filtered
}
