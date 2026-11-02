const {shippingFee}=require('./shipping.cjs');exports.checkoutTotal=n=>n+shippingFee(n);
