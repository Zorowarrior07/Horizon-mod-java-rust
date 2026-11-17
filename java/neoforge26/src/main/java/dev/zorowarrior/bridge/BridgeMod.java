package dev.zorowarrior.bridge;

import net.minecraft.resources.Identifier;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.function.UnaryOperator;

/**
 * ZOROWARRIOR HORIZON BRIDGE — Minecraft 26.x, NeoForge.
 *
 * <p>Same content as the 1.21.x build, against the 26.x API. 26.x is a separate
 * project because it differs in ways that stop the 1.21.x source compiling at
 * all — every one of these was read off the 26.1 jar, not remembered:
 *
 * <ul>
 *   <li>{@code net.minecraft.resources.ResourceLocation} is <b>gone</b>;
 *       26.x uses {@code net.minecraft.resources.Identifier}.</li>
 *   <li>A block needs its own id before construction, or the constructor throws
 *       {@code NullPointerException: Block id not set}. NeoForge's
 *       {@code registerSimpleBlock(name, ...)} supplies it for you — which is why
 *       this uses that overload instead of {@code register(...)}, where the
 *       factory has to build a {@code BlockBehaviour.Properties} that already
 *       knows its key.</li>
 *   <li>26.1 requires Java 25 (class file version 69).</li>
 * </ul>
 *
 * <p>Mojang publishes no mappings for 26.x at all, so the only supported build
 * route is NeoForge's NeoForm pipeline. There is deliberately no Fabric 26.x jar.
 */
@Mod(BridgeMod.MOD_ID)
public final class BridgeMod {

    public static final String MOD_ID = "zoro_bridge";
    public static final Logger LOGGER = LoggerFactory.getLogger("ZoroHorizonBridge");

    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(MOD_ID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(MOD_ID);

    /** The one place the 26.x id type appears. */
    public static Identifier id(String path) {
        return Identifier.fromNamespaceAndPath(MOD_ID, path);
    }

    /** Block properties, shared by the block and its block item. */
    private static final UnaryOperator<net.minecraft.world.level.block.state.BlockBehaviour.Properties>
            BEACON_PROPERTIES = props -> props
                    .strength(3.5F, 6.0F)
                    .lightLevel(state -> 12)
                    .sound(net.minecraft.world.level.block.SoundType.STONE)
                    .requiresCorrectToolForDrops();

    public static final DeferredBlock<Block> BRIDGE_BEACON =
            BLOCKS.registerSimpleBlock("bridge_beacon", BEACON_PROPERTIES);

    public static final DeferredItem<Item> BRIDGE_CHIP =
            ITEMS.registerSimpleItem("bridge_chip", new Item.Properties());

    public BridgeMod(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        // The block item gets the same id-then-construct treatment.
        ITEMS.registerSimpleBlockItem(BRIDGE_BEACON, new Item.Properties());
        LOGGER.info("[ZoroHorizonBridge] NeoForge 26.x build initialised: {} + {}",
                id("bridge_beacon"), id("bridge_chip"));
    }
}
