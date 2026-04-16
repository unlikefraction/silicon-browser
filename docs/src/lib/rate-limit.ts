type RateLimitResult = {
  success: boolean;
  limit: number;
  remaining: number;
  reset: number;
};

type Bucket = {
  count: number;
  resetAt: number;
};

class InMemoryRateLimiter {
  private readonly buckets = new Map<string, Bucket>();

  constructor(
    private readonly limit: number,
    private readonly windowMs: number,
  ) {}

  async limitRequest(identifier: string): Promise<RateLimitResult> {
    const now = Date.now();
    const bucket = this.buckets.get(identifier);

    if (!bucket || bucket.resetAt <= now) {
      const nextBucket = {
        count: 1,
        resetAt: now + this.windowMs,
      };
      this.buckets.set(identifier, nextBucket);
      return {
        success: true,
        limit: this.limit,
        remaining: this.limit - 1,
        reset: nextBucket.resetAt,
      };
    }

    if (bucket.count >= this.limit) {
      return {
        success: false,
        limit: this.limit,
        remaining: 0,
        reset: bucket.resetAt,
      };
    }

    bucket.count += 1;
    return {
      success: true,
      limit: this.limit,
      remaining: this.limit - bucket.count,
      reset: bucket.resetAt,
    };
  }
}

const minuteLimiter = new InMemoryRateLimiter(
  Number(process.env.RATE_LIMIT_PER_MINUTE) || 10,
  60_000,
);
const dailyLimiter = new InMemoryRateLimiter(
  Number(process.env.RATE_LIMIT_PER_DAY) || 100,
  86_400_000,
);

export const minuteRateLimit = {
  limit: (identifier: string) => minuteLimiter.limitRequest(identifier),
};

export const dailyRateLimit = {
  limit: (identifier: string) => dailyLimiter.limitRequest(identifier),
};
